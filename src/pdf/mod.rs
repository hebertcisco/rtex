//! Writing a [`Document`] as PDF, as pdfTeX would: embedded Type 1 subsets, pdfTeX's
//! objects, forms, images, annotations, destinations, outlines and document information.

mod content;
mod file;
mod fonts;
mod images;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use content::{FontShape, Resources};
use file::{PdfFile, bp, escaped, pdf_str, real};

use crate::document::{Area, DestName, Document, DrawOp, FontId, PdfAction, PdfOp, PdfTarget};
use crate::fonts::library::FontLibrary;
use crate::images::Image;
use crate::tex::backend::DestKind;
use crate::units::Scaled;

const PRODUCER: &str = concat!("rtex-", env!("CARGO_PKG_VERSION"));

/// A pdfTeX object with its token lists turned into text.
#[derive(Clone, Debug)]
pub enum JobObject {
    /// `\pdfobj reserveobjnum` never filled, or an annotation number.
    Reserved,
    /// `\pdfobj`.
    Raw {
        /// Written as a stream.
        stream: bool,
        /// The stream's extra dictionary entries.
        attr: String,
        /// The object text or stream data.
        data: Vec<u8>,
    },
    /// `\pdfxform`.
    Form {
        /// Its number among forms (`/Fm1`).
        index: usize,
        /// Width.
        width: Scaled,
        /// Height.
        height: Scaled,
        /// Depth.
        depth: Scaled,
        /// Extra dictionary entries.
        attr: String,
        /// Extra resources.
        resources: String,
        /// The form's content.
        ops: Vec<DrawOp>,
    },
    /// `\pdfximage`.
    Image {
        /// Its number among images (`/Im1`).
        index: usize,
        /// The image.
        image: Arc<Image>,
        /// Extra dictionary entries.
        attr: String,
    },
}

/// An outline entry with its strings expanded.
#[derive(Clone, Debug)]
pub struct JobOutline {
    /// `attr{...}`.
    pub attr: Option<String>,
    /// The action.
    pub action: PdfAction,
    /// `count n`.
    pub count: i32,
    /// The title.
    pub title: String,
    /// The parent entry.
    pub parent: Option<usize>,
}

/// Everything pdfTeX's primitives contributed to the document, ready for the writer.
#[derive(Clone, Debug, Default)]
pub struct PdfJob {
    /// Objects by pdfTeX number.
    pub objects: BTreeMap<usize, JobObject>,
    /// Numbers up to this one were handed out during the run.
    pub last_number: usize,
    /// Object numbers of pages, by page number.
    pub page_objects: HashMap<usize, usize>,
    /// Object numbers of fonts referenced by `\pdffontobjnum`.
    pub font_objects: HashMap<usize, usize>,
    /// `\pdfinfo`.
    pub info: Vec<String>,
    /// `\pdfcatalog`.
    pub catalog: Vec<String>,
    /// `\pdfcatalog openaction`.
    pub open_action: Option<PdfAction>,
    /// `\pdfnames`.
    pub names: Vec<String>,
    /// `\pdftrailer`.
    pub trailer: Vec<String>,
    /// `\pdftrailerid`.
    pub trailer_id: Option<String>,
    /// Outline entries.
    pub outlines: Vec<JobOutline>,
    /// `\pdfpagesattr`.
    pub pages_attr: String,
    /// `\pdfcompresslevel`.
    pub compress_level: i32,
    /// `\pdfdecimaldigits`.
    pub decimal_digits: i32,
    /// `\pdfminorversion`.
    pub minor_version: i32,
    /// `\pdfgentounicode`.
    pub gen_tounicode: bool,
    /// `\pdfinfoomitdate`.
    pub info_omit_date: bool,
    /// `\pdfomitcharset`.
    pub omit_charset: bool,
    /// `\pdfomitinfodict`.
    pub omit_info: bool,
    /// `\pdfglyphtounicode`.
    pub to_unicode: HashMap<String, String>,
    /// `\pdffontattr`.
    pub font_attrs: HashMap<usize, String>,
    /// The creation date.
    pub date: String,
}

/// What the writer reports besides the file: pdfTeX's warnings about missing destinations.
#[derive(Clone, Debug, Default)]
pub struct PdfOutput {
    /// The PDF file.
    pub bytes: Vec<u8>,
    /// Warnings to print in the transcript.
    pub warnings: Vec<String>,
}

/// The resource name of a form or image object: `/Fm<index>` or `/Im<index>`.
pub(crate) fn xobject_name(job: &PdfJob, n: usize) -> String {
    match job.objects.get(&n) {
        Some(JobObject::Form { index, .. }) => format!("Fm{index}"),
        Some(JobObject::Image { index, .. }) => format!("Im{index}"),
        _ => format!("X{n}"),
    }
}

struct Writer<'a> {
    file: PdfFile,
    job: &'a PdfJob,
    digits: usize,
    page_objects: HashMap<usize, usize>,
    num_dests: BTreeMap<i32, usize>,
    name_dests: BTreeMap<String, usize>,
    defined: HashMap<usize, String>,
}

impl Writer<'_> {
    fn num(&self, v: f64) -> String {
        real(v, self.digits)
    }

    fn page_object(&mut self, n: i32) -> usize {
        let n = usize::try_from(n).unwrap_or(1).max(1);
        if let Some(&o) = self.page_objects.get(&n) {
            return o;
        }
        let o = self.file.alloc();
        self.page_objects.insert(n, o);
        o
    }

    fn num_dest(&mut self, n: i32) -> usize {
        if let Some(&o) = self.num_dests.get(&n) {
            return o;
        }
        let o = self.file.alloc();
        self.num_dests.insert(n, o);
        o
    }

    fn name_dest(&mut self, name: &str) -> usize {
        if let Some(&o) = self.name_dests.get(name) {
            return o;
        }
        let o = self.file.alloc();
        self.name_dests.insert(name.to_owned(), o);
        o
    }

    /// `write_action`.
    fn action(&mut self, a: &PdfAction) -> String {
        if let PdfTarget::User(s) = &a.target {
            return s.clone();
        }
        let mut s = String::from("<< ");
        if let Some(f) = &a.file {
            let _ = write!(s, "/F {} ", pdf_str(f));
            match a.new_window {
                1 => s.push_str("/NewWindow true "),
                2 => s.push_str("/NewWindow false "),
                _ => {}
            }
        }
        match &a.target {
            PdfTarget::Page(n, spec) => {
                if a.file.is_none() {
                    let p = self.page_object(*n);
                    let _ = write!(s, "/S /GoTo /D [{p} 0 R {spec}]");
                } else {
                    let _ = write!(s, "/S /GoToR /D [{} {spec}]", n - 1);
                }
            }
            PdfTarget::Name(name) => {
                if a.file.is_none() {
                    self.name_dest(name);
                    s.push_str("/S /GoTo ");
                } else {
                    s.push_str("/S /GoToR ");
                }
                let _ = write!(s, "/D {}", pdf_str(name));
            }
            PdfTarget::Num(n) => {
                let d = self.num_dest(*n);
                let _ = write!(s, "/S /GoTo /D {d} 0 R");
            }
            PdfTarget::ThreadName(name) => {
                let _ = write!(s, "/S /Thread /D {}", pdf_str(name));
            }
            PdfTarget::ThreadNum(n) => {
                let _ = write!(s, "/S /Thread /D {n}");
            }
            PdfTarget::User(_) => {}
        }
        s.push_str(" >>");
        s
    }

    fn rect(&self, a: &Area, height: Scaled) -> String {
        format!(
            "[{} {} {} {}]",
            self.num(bp(a.left)),
            self.num(bp(height - a.bottom)),
            self.num(bp(a.right)),
            self.num(bp(height - a.top))
        )
    }

    fn resources(&self, r: &Resources, fonts: &BTreeMap<FontId, usize>, extra: &str) -> String {
        let mut s = String::from("<<\n/ProcSet [/PDF /Text /ImageB /ImageC /ImageI]\n");
        if !r.fonts.is_empty() {
            s.push_str("/Font <<");
            for f in &r.fonts {
                if let Some(id) = fonts.get(f) {
                    let _ = write!(s, " /F{f} {id} 0 R");
                }
            }
            s.push_str(" >>\n");
        }
        if !r.forms.is_empty() || !r.images.is_empty() {
            s.push_str("/XObject <<");
            for n in r.forms.iter().chain(&r.images) {
                let _ = write!(s, " /{} {n} 0 R", xobject_name(self.job, *n));
            }
            s.push_str(" >>\n");
        }
        if !extra.is_empty() {
            s.push_str(extra);
            s.push('\n');
        }
        s.push_str(">>");
        s
    }

    fn dest_array(&self, page: usize, kind: DestKind, area: &Area, height: Scaled) -> String {
        let x = self.num(bp(area.left));
        let top = self.num(bp(height - area.top));
        match kind {
            DestKind::Xyz(zoom) => {
                let z = zoom.map_or("null".to_owned(), |z| real(f64::from(z) / 1000.0, 3));
                format!("[{page} 0 R /XYZ {x} {top} {z}]")
            }
            DestKind::Fit => format!("[{page} 0 R /Fit]"),
            DestKind::FitB => format!("[{page} 0 R /FitB]"),
            DestKind::FitH => format!("[{page} 0 R /FitH {top}]"),
            DestKind::FitBH => format!("[{page} 0 R /FitBH {top}]"),
            DestKind::FitV => format!("[{page} 0 R /FitV {x}]"),
            DestKind::FitBV => format!("[{page} 0 R /FitBV {x}]"),
            DestKind::FitR => format!(
                "[{page} 0 R /FitR {x} {} {} {top}]",
                self.num(bp(height - area.bottom)),
                self.num(bp(area.right))
            ),
        }
    }
}

/// Serializes `document` as a PDF file, with pdfTeX's `job` and fonts from `library`.
pub fn write_pdf(document: &Document, job: &PdfJob, library: &FontLibrary) -> PdfOutput {
    let digits = job.decimal_digits.clamp(0, 4) as usize;
    let mut w = Writer {
        file: PdfFile::new(job.last_number, job.compress_level),
        job,
        digits,
        page_objects: job.page_objects.clone(),
        num_dests: BTreeMap::new(),
        name_dests: BTreeMap::new(),
        defined: HashMap::new(),
    };
    let mut warnings = Vec::new();
    let catalog = w.file.alloc();
    let tree = w.file.alloc();
    let info = (!job.omit_info).then(|| w.file.alloc());
    let pages: Vec<(usize, usize)> = (1..=document.pages().len())
        .map(|n| {
            let p = w.page_object(n as i32);
            (p, w.file.alloc())
        })
        .collect();

    let mut shapes = BTreeMap::new();
    for (id, f) in document.fonts() {
        if let Some(e) = library.entry(&f.name) {
            shapes.insert(
                id,
                FontShape {
                    slant: e.slant,
                    extend: e.extend,
                },
            );
        }
    }
    let form_ops = job.objects.values().filter_map(|o| match o {
        JobObject::Form { ops, .. } => Some(ops.as_slice()),
        _ => None,
    });
    let page_ops = document.pages().iter().map(|p| p.ops.as_slice());
    let font_options = fonts::FontOptions {
        gen_tounicode: job.gen_tounicode,
        to_unicode: &job.to_unicode,
        attrs: &job.font_attrs,
        objects: &job.font_objects,
        omit_charset: job.omit_charset,
    };
    let font_ids = fonts::write_fonts(&mut w.file, document, page_ops.chain(form_ops), library, &font_options);

    for (&n, o) in &job.objects {
        match o {
            JobObject::Reserved => {}
            JobObject::Raw { stream, attr, data } => {
                if *stream {
                    w.file.stream(n, attr, data);
                } else {
                    w.file.object(n, data.clone());
                }
            }
            JobObject::Form {
                index: _,
                width,
                height,
                depth,
                attr,
                resources,
                ops,
            } => {
                let h = height + depth;
                let (data, res) = content::render(document, &shapes, ops, h, digits, job);
                let dict = format!(
                    "/Type /XObject\n/Subtype /Form\n/BBox [0 0 {} {}]\n/FormType 1\n/Matrix [1 0 0 1 0 0]\n/Resources {}\n{attr}",
                    w.num(bp(*width)),
                    w.num(bp(h)),
                    w.resources(&res, &font_ids, resources)
                );
                w.file.stream(n, &dict, &data);
            }
            JobObject::Image { image, attr, .. } => images::write_image(&mut w.file, n, image, attr),
        }
    }

    let mut dests: Vec<(DestName, usize, DestKind, Area, Scaled)> = Vec::new();
    for (page, &(page_id, content_id)) in document.pages().iter().zip(&pages) {
        let (data, res) = content::render(document, &shapes, &page.ops, page.height, digits, job);
        w.file.stream(content_id, "", &data);
        let mut annots = Vec::new();
        for op in &page.ops {
            let DrawOp::Pdf { op, .. } = op else { continue };
            match op {
                PdfOp::Annot { obj, area, data } => {
                    let r = w.rect(area, page.height);
                    w.file.object(*obj, format!("<<\n/Type /Annot\n/Rect {r}\n{data}\n>>"));
                    annots.push(*obj);
                }
                PdfOp::Link {
                    obj,
                    area,
                    attr,
                    action,
                } => {
                    let n = if *obj == 0 || w.file.has(*obj) {
                        w.file.alloc()
                    } else {
                        *obj
                    };
                    let r = w.rect(area, page.height);
                    let user = matches!(action.target, PdfTarget::User(_));
                    let a = w.action(action);
                    let mut d = String::from("<<\n/Type /Annot\n");
                    if !user {
                        d.push_str("/Subtype /Link\n");
                    }
                    if let Some(attr) = attr {
                        d.push_str(attr);
                        d.push('\n');
                    }
                    let _ = writeln!(d, "/Rect {r}");
                    if !user {
                        d.push_str("/A ");
                    }
                    d.push_str(&a);
                    d.push_str("\n>>");
                    w.file.object(n, d);
                    annots.push(n);
                }
                PdfOp::Dest { name, kind, area } => dests.push((name.clone(), page_id, *kind, *area, page.height)),
                _ => {}
            }
        }
        let mut d = format!(
            "<<\n/Type /Page\n/Contents {content_id} 0 R\n/Resources {}\n/MediaBox [0 0 {} {}]\n/Parent {tree} 0 R\n",
            w.resources(&res, &font_ids, &page.resources),
            w.num(bp(page.width)),
            w.num(bp(page.height))
        );
        if !annots.is_empty() {
            let list: Vec<String> = annots.iter().map(|a| format!("{a} 0 R")).collect();
            let _ = writeln!(d, "/Annots [{}]", list.join(" "));
        }
        if !page.attr.is_empty() {
            d.push_str(&page.attr);
            d.push('\n');
        }
        d.push_str(">>");
        w.file.object(page_id, d);
    }

    for (name, page, kind, area, height) in &dests {
        let array = w.dest_array(*page, *kind, area, *height);
        let obj = match name {
            DestName::Num(n) => w.num_dest(*n),
            DestName::Name(s) => w.name_dest(s),
        };
        w.defined.insert(obj, array.clone());
        w.file.object(obj, array);
    }

    let outlines = write_outlines(&mut w);
    let open_action = job.open_action.as_ref().map(|a| w.action(a));
    let first_page = pages.first().map_or(0, |p| p.0);
    let referenced: Vec<(String, usize)> = w
        .num_dests
        .iter()
        .map(|(n, o)| (format!("num{n}"), *o))
        .chain(w.name_dests.iter().map(|(n, o)| (format!("name{{{n}}}"), *o)))
        .collect();
    for (label, obj) in referenced {
        if !w.defined.contains_key(&obj) {
            warnings.push(format!(
                "pdfTeX warning (dest): {label} has been referenced but does not exist, replaced by a fixed one"
            ));
            w.file.object(obj, format!("[{first_page} 0 R /Fit]"));
        }
    }

    let kids: Vec<String> = pages.iter().map(|(p, _)| format!("{p} 0 R")).collect();
    let mut tree_dict = format!("<<\n/Type /Pages\n/Count {}\n/Kids [{}]\n", pages.len(), kids.join(" "));
    if !job.pages_attr.is_empty() {
        tree_dict.push_str(&job.pages_attr);
        tree_dict.push('\n');
    }
    tree_dict.push_str(">>");
    w.file.object(tree, tree_dict);

    let mut cat = format!("<<\n/Type /Catalog\n/Pages {tree} 0 R\n");
    if let Some(o) = outlines {
        let _ = writeln!(cat, "/Outlines {o} 0 R");
    }
    if !w.name_dests.is_empty() || !job.names.is_empty() {
        let names = w.file.alloc();
        let mut nd = String::from("<<\n");
        if !w.name_dests.is_empty() {
            let tree_obj = w.file.alloc();
            let entries: Vec<String> = w
                .name_dests
                .iter()
                .map(|(k, v)| format!("{} {v} 0 R", escaped(k)))
                .collect();
            let first = w.name_dests.keys().next().cloned().unwrap_or_default();
            let last = w.name_dests.keys().last().cloned().unwrap_or_default();
            w.file.object(
                tree_obj,
                format!(
                    "<<\n/Names [{}]\n/Limits [{} {}]\n>>",
                    entries.join(" "),
                    escaped(&first),
                    escaped(&last)
                ),
            );
            let _ = writeln!(nd, "/Dests {tree_obj} 0 R");
        }
        for n in &job.names {
            nd.push_str(n);
            nd.push('\n');
        }
        nd.push_str(">>");
        w.file.object(names, nd);
        let _ = writeln!(cat, "/Names {names} 0 R");
    }
    if let Some(a) = open_action {
        let _ = writeln!(cat, "/OpenAction {a}");
    }
    for c in &job.catalog {
        cat.push_str(c);
        cat.push('\n');
    }
    cat.push_str(">>");
    w.file.object(catalog, cat);

    if let Some(i) = info {
        let user = job.info.join("\n");
        let mut d = String::from("<<\n");
        if !user.is_empty() {
            d.push_str(&user);
            d.push('\n');
        }
        let has = |k: &str| user.contains(k);
        if !has("/Producer") {
            let _ = writeln!(d, "/Producer ({PRODUCER})");
        }
        if !has("/Creator") {
            d.push_str("/Creator (TeX)\n");
        }
        if !job.info_omit_date {
            if !has("/CreationDate") {
                let _ = writeln!(d, "/CreationDate ({})", job.date);
            }
            if !has("/ModDate") {
                let _ = writeln!(d, "/ModDate ({})", job.date);
            }
        }
        if !has("/Trapped") {
            d.push_str("/Trapped /False\n");
        }
        d.push_str(">>");
        w.file.object(i, d);
    }

    let mut trailer = format!("/Root {catalog} 0 R\n");
    if let Some(i) = info {
        let _ = writeln!(trailer, "/Info {i} 0 R");
    }
    match &job.trailer_id {
        Some(id) if id.trim().is_empty() => {}
        Some(id) => {
            let _ = writeln!(trailer, "/ID {id}");
        }
        None => {
            let seed = format!("{}{}{}", job.date, document.pages().len(), PRODUCER);
            let id = crate::tex::md5_hex(seed.as_bytes());
            let _ = writeln!(trailer, "/ID [<{id}> <{id}>]");
        }
    }
    for t in &job.trailer {
        trailer.push_str(t);
        trailer.push('\n');
    }
    let minor = if job.minor_version > 0 { job.minor_version } else { 4 };
    PdfOutput {
        bytes: w.file.finish(minor, trailer.trim_end()),
        warnings,
    }
}

fn write_outlines(w: &mut Writer) -> Option<usize> {
    let job = w.job;
    if job.outlines.is_empty() {
        return None;
    }
    let root = w.file.alloc();
    let ids: Vec<usize> = job.outlines.iter().map(|_| w.file.alloc()).collect();
    let children = |parent: Option<usize>| -> Vec<usize> {
        job.outlines
            .iter()
            .enumerate()
            .filter(|(_, o)| o.parent == parent)
            .map(|(k, _)| k)
            .collect()
    };
    fn open_subentries(k: usize, job: &PdfJob, counts: &mut [i32]) -> i32 {
        let mut n = 0;
        for (c, o) in job.outlines.iter().enumerate() {
            if o.parent == Some(k) {
                n += 1;
                let sub = open_subentries(c, job, counts);
                if job.outlines[c].count > 0 {
                    n += sub;
                }
            }
        }
        counts[k] = if job.outlines[k].count > 0 { n } else { -n };
        n
    }
    let mut counts = vec![0; job.outlines.len()];
    let mut total = 0;
    for (k, o) in job.outlines.iter().enumerate() {
        if o.parent.is_none() {
            total += 1;
            let sub = open_subentries(k, job, &mut counts);
            if o.count > 0 {
                total += sub;
            }
        }
    }
    for (k, o) in job.outlines.iter().enumerate() {
        let siblings = children(o.parent);
        let pos = siblings.iter().position(|&s| s == k).unwrap_or(0);
        let kids = children(Some(k));
        let parent = o.parent.map_or(root, |p| ids[p]);
        let action = w.action(&o.action);
        let mut d = format!("<<\n/Title {}\n/A {action}\n/Parent {parent} 0 R\n", pdf_str(&o.title));
        if pos > 0 {
            let _ = writeln!(d, "/Prev {} 0 R", ids[siblings[pos - 1]]);
        }
        if let Some(&next) = siblings.get(pos + 1) {
            let _ = writeln!(d, "/Next {} 0 R", ids[next]);
        }
        if let (Some(&f), Some(&l)) = (kids.first(), kids.last()) {
            let _ = writeln!(d, "/First {} 0 R\n/Last {} 0 R", ids[f], ids[l]);
        }
        if counts[k] != 0 {
            let _ = writeln!(d, "/Count {}", counts[k]);
        }
        if let Some(a) = &o.attr {
            d.push_str(a);
            d.push('\n');
        }
        d.push_str(">>");
        w.file.object(ids[k], d);
    }
    let top = children(None);
    let (first, last) = (ids[*top.first()?], ids[*top.last()?]);
    w.file.object(
        root,
        format!("<<\n/Type /Outlines\n/First {first} 0 R\n/Last {last} 0 R\n/Count {total}\n>>"),
    );
    Some(root)
}
