//! EPUB 3 output: reflowable XHTML from the shipped pages, and a fixed layout
//! with one SVG content document per page.

mod zip;

use std::fmt::Write as _;

use crate::document::{Document, DrawOp, Page};
use crate::units::Scaled;

/// What a reflowable page needs to be a book.
#[derive(Clone, Debug)]
pub struct Metadata {
    /// `dc:title`.
    pub title: String,
    /// `dc:creator`.
    pub author: Option<String>,
    /// `dc:language`, a BCP 47 tag.
    pub language: String,
    /// `dc:identifier`.
    pub identifier: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Metadata {
            title: "Untitled".to_owned(),
            author: None,
            language: "en".to_owned(),
            identifier: "urn:rtex:document".to_owned(),
        }
    }
}

/// How the book reflows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Text reflows in the reader; the default.
    #[default]
    Reflow,
    /// Every shipped page is one pre-paginated SVG page.
    Fixed,
}

/// What to write.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Reflowable or pre-paginated.
    pub layout: Layout,
    /// The package metadata.
    pub metadata: Metadata,
}

/// The fixed timestamp `dcterms:modified` carries, kept deterministic.
const MODIFIED: &str = "2000-01-01T00:00:00Z";

/// Reads back the entries of an EPUB built here.
pub fn entries(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    zip::read(archive)
}

/// Builds an EPUB 3 archive from shipped pages.
pub fn build(document: &Document, options: &Options) -> Vec<u8> {
    let pages = document.pages();
    let fixed = options.layout == Layout::Fixed;
    let mut entries: Vec<(String, Vec<u8>, bool)> = vec![
        ("mimetype".to_owned(), b"application/epub+zip".to_vec(), false),
        ("META-INF/container.xml".to_owned(), container().into_bytes(), true),
    ];
    entries.push(("OEBPS/style.css".to_owned(), css(fixed).into_bytes(), true));
    for (index, page) in pages.iter().enumerate() {
        let (name, data) = if fixed {
            (
                format!("OEBPS/page{}.svg", index + 1),
                fixed_page(document, page, index + 1),
            )
        } else {
            (
                format!("OEBPS/page{}.xhtml", index + 1),
                reflow_page(document, page, index + 1, options),
            )
        };
        entries.push((name, data.into_bytes(), true));
    }
    entries.push((
        "OEBPS/nav.xhtml".to_owned(),
        nav(pages.len(), &options.metadata, fixed).into_bytes(),
        true,
    ));
    entries.push((
        "OEBPS/toc.ncx".to_owned(),
        ncx(pages.len(), &options.metadata, fixed).into_bytes(),
        true,
    ));
    entries.push((
        "OEBPS/content.opf".to_owned(),
        opf(pages.len(), options).into_bytes(),
        true,
    ));
    zip::archive(&entries)
}

fn container() -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
        "<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n",
        "  <rootfiles>\n",
        "    <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n",
        "  </rootfiles>\n",
        "</container>\n"
    )
    .to_owned()
}

fn css(fixed: bool) -> String {
    if fixed {
        "html, body { margin: 0; padding: 0; }\nsvg { display: block; }\n".to_owned()
    } else {
        concat!(
            "body { font-family: serif; line-height: 1.5; margin: 5%; }\n",
            "p { margin: 0 0 0.8em 0; text-align: justify; }\n",
            "h1, h2, h3 { font-family: sans-serif; line-height: 1.2; }\n",
            "pre, code { font-family: monospace; }\n",
            "table { border-collapse: collapse; }\n",
            "td, th { border: 1px solid #999; padding: 0.2em 0.4em; }\n"
        )
        .to_owned()
    }
}

fn nav(pages: usize, metadata: &Metadata, fixed: bool) -> String {
    let mut out = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
        "<!DOCTYPE html>\n",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"",
    ));
    let _ = writeln!(
        out,
        " xml:lang=\"{}\" lang=\"{}\">",
        escape(&metadata.language),
        escape(&metadata.language)
    );
    out.push_str("<head><title>Contents</title></head>\n<body>\n");
    out.push_str("<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol>\n");
    for page in 1..=pages {
        let extension = if fixed { "svg" } else { "xhtml" };
        let _ = writeln!(out, "<li><a href=\"page{page}.{extension}\">Page {page}</a></li>");
    }
    out.push_str("</ol></nav>\n</body>\n</html>\n");
    out
}

fn ncx(pages: usize, metadata: &Metadata, fixed: bool) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n");
    out.push_str("<head>\n");
    let _ = writeln!(
        out,
        "<meta name=\"dtb:uid\" content=\"{}\"/>",
        escape(&metadata.identifier)
    );
    out.push_str("<meta name=\"dtb:depth\" content=\"1\"/>\n");
    out.push_str("<meta name=\"dtb:totalPageCount\" content=\"0\"/>\n");
    out.push_str("<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n");
    out.push_str("</head>\n<docTitle>");
    let _ = write!(out, "<text>{}</text>", escape(&metadata.title));
    out.push_str("</docTitle>\n<navMap>\n");
    for page in 1..=pages {
        let extension = if fixed { "svg" } else { "xhtml" };
        let _ = writeln!(
            out,
            "<navPoint id=\"p{page}\" playOrder=\"{page}\"><navLabel><text>Page {page}</text></navLabel><content src=\"page{page}.{extension}\"/></navPoint>"
        );
    }
    out.push_str("</navMap>\n</ncx>\n");
    out
}

fn opf(pages: usize, options: &Options) -> String {
    let fixed = options.layout == Layout::Fixed;
    let metadata = &options.metadata;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    let _ = writeln!(
        out,
        "<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"pub-id\" xml:lang=\"{}\">",
        escape(&metadata.language)
    );
    out.push_str("<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n");
    let _ = writeln!(
        out,
        "<dc:identifier id=\"pub-id\">{}</dc:identifier>",
        escape(&metadata.identifier)
    );
    let _ = writeln!(out, "<dc:title>{}</dc:title>", escape(&metadata.title));
    let _ = writeln!(out, "<dc:language>{}</dc:language>", escape(&metadata.language));
    if let Some(author) = &metadata.author {
        let _ = writeln!(out, "<dc:creator>{}</dc:creator>", escape(author));
    }
    let _ = writeln!(out, "<meta property=\"dcterms:modified\">{MODIFIED}</meta>");
    if fixed {
        out.push_str("<meta property=\"rendition:layout\">pre-paginated</meta>\n");
    }
    out.push_str("</metadata>\n<manifest>\n");
    out.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    out.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    out.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    for page in 1..=pages {
        if fixed {
            let _ = writeln!(
                out,
                "<item id=\"p{page}\" href=\"page{page}.svg\" media-type=\"image/svg+xml\"/>"
            );
        } else {
            let _ = writeln!(
                out,
                "<item id=\"p{page}\" href=\"page{page}.xhtml\" media-type=\"application/xhtml+xml\"/>"
            );
        }
    }
    out.push_str("</manifest>\n<spine toc=\"ncx\">\n");
    for page in 1..=pages {
        let _ = writeln!(out, "<itemref idref=\"p{page}\"/>");
    }
    out.push_str("</spine>\n</package>\n");
    out
}

struct Run {
    x: Scaled,
    end: Scaled,
    text: String,
    style: Style,
    size: Scaled,
    math: bool,
    hidden: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    mono: bool,
}

struct Line {
    baseline: Scaled,
    runs: Vec<Run>,
}

/// Decodes a run of text-font codes. OT1 text fonts keep quotes, dashes,
/// ligatures and accents in the ASCII and control ranges; typewriter faces use
/// ASCII as-is, and other fonts use Latin-1.
fn decode(codes: &[u32], font: &str) -> String {
    let lower = font.to_ascii_lowercase();
    let ot1 = lower.starts_with("cm") || lower.starts_with("lm");
    let mono = lower.starts_with("cmtt") || lower.starts_with("lmtt");
    let mut out = String::with_capacity(codes.len());
    for &code in codes {
        if ot1 && !mono {
            let replacement = match code {
                11 => Some("ff"),
                12 => Some("fi"),
                13 => Some("fl"),
                14 => Some("ffi"),
                15 => Some("ffl"),
                16 => Some("ı"),
                17 => Some("ȷ"),
                18 => Some("`"),
                19 => Some("´"),
                20 => Some("ˇ"),
                21 => Some("˘"),
                22 => Some("¯"),
                23 => Some("˚"),
                24 => Some("¸"),
                25 => Some("ß"),
                26 => Some("æ"),
                27 => Some("œ"),
                28 => Some("ø"),
                29 => Some("Æ"),
                30 => Some("Œ"),
                31 => Some("Ø"),
                34 => Some("”"),
                39 => Some("’"),
                60 => Some("¡"),
                62 => Some("¿"),
                92 => Some("“"),
                94 => Some("ˆ"),
                95 => Some("˙"),
                96 => Some("‘"),
                123 => Some("–"),
                124 => Some("—"),
                125 => Some("˝"),
                126 => Some("˜"),
                127 => Some("¨"),
                _ => None,
            };
            if let Some(text) = replacement {
                out.push_str(text);
                continue;
            }
        }
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        if c == '\r' || c == '\n' || c == '\t' {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out
}

fn style_of(name: &str) -> Style {
    let name = name.to_ascii_lowercase();
    Style {
        bold: ["cmbx", "cmb", "bx", "bold", "ebgaramond-bold"]
            .iter()
            .any(|k| name.contains(k)),
        italic: ["cmti", "cmsl", "ti", "sl", "italic", "oblique"]
            .iter()
            .any(|k| name.contains(k)),
        mono: ["cmtt", "tt", "mono", "courier"].iter().any(|k| name.contains(k)),
    }
}

fn page_lines(document: &Document, page: &Page) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for op in &page.ops {
        let DrawOp::Text {
            font,
            x,
            baseline,
            codes,
            advances,
        } = op
        else {
            continue;
        };
        let name = document.font(*font).map_or("", |f| f.name.as_str());
        let math = is_math_font(name);
        let text = decode(codes, name);
        if text.trim().is_empty() {
            continue;
        }
        let size = document.font(*font).map_or(655_360, |f| f.size);
        let width: Scaled = advances.iter().sum();
        let run = Run {
            x: *x,
            end: *x + width,
            text,
            style: style_of(name),
            size,
            math,
            hidden: false,
        };
        match lines.iter_mut().find(|line| (line.baseline - baseline).abs() <= 1) {
            Some(line) => line.runs.push(run),
            None => lines.push(Line {
                baseline: *baseline,
                runs: vec![run],
            }),
        }
    }
    for line in &mut lines {
        line.runs.sort_by_key(|run| run.x);
        hide_math(&mut line.runs);
    }
    lines.sort_by_key(|line| line.baseline);
    lines
}

fn median_size(document: &Document, page: &Page) -> Scaled {
    let mut sizes: Vec<Scaled> = page
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text { font, .. } => document.font(*font).map(|f| f.size),
            _ => None,
        })
        .collect();
    if sizes.is_empty() {
        return 655_360;
    }
    sizes.sort_unstable();
    sizes[sizes.len() / 2]
}

fn reflow_page(document: &Document, page: &Page, number: usize, options: &Options) -> String {
    let body = median_size(document, page);
    let lines = page_lines(document, page);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n");
    let _ = writeln!(
        out,
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{}\" lang=\"{}\">",
        escape(&options.metadata.language),
        escape(&options.metadata.language)
    );
    let _ = writeln!(
        out,
        "<head><title>Page {number}</title><link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/></head>"
    );
    out.push_str("<body>\n");
    let mut paragraph: Vec<&Line> = Vec::new();
    let mut previous: Option<&Line> = None;
    for line in &lines {
        if is_page_number(line, page) || is_math_line(line) {
            continue;
        }
        let new_block = previous.is_some_and(|last| {
            let gap = line.baseline - last.baseline;
            gap > body * 3 / 2
        });
        if new_block && !paragraph.is_empty() {
            emit_paragraph(&mut out, &paragraph);
            paragraph.clear();
        }
        paragraph.push(line);
        previous = Some(line);
    }
    if !paragraph.is_empty() {
        emit_paragraph(&mut out, &paragraph);
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn is_math_font(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["cmmi", "cmsy", "cmex", "msam", "msbm", "rsfs", "eufm"]
        .iter()
        .any(|k| name.starts_with(k))
}

fn is_page_number(line: &Line, page: &Page) -> bool {
    line.baseline > page.height * 3 / 4
        && line
            .runs
            .iter()
            .all(|run| run.text.chars().all(|c| c.is_ascii_digit() || c == ' '))
}

fn line_text(line: &Line) -> String {
    line.runs
        .iter()
        .filter(|run| !run.hidden)
        .map(|run| run.text.as_str())
        .collect()
}

fn is_math_line(line: &Line) -> bool {
    let text = line_text(line);
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
    letters == 0 && digits > 0
}

fn looks_math(text: &str) -> bool {
    !text.trim().is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || "()[]{}<>+-=*,./|!:;'^_ \u{2212}†‡§¶©®±×÷≤≥≠≈∞∑∏∫√".contains(c))
}

fn hide_math(runs: &mut [Run]) {
    let mut index = 0;
    while index < runs.len() {
        if !runs[index].math {
            index += 1;
            continue;
        }
        runs[index].hidden = true;
        let mut left = index;
        while left > 0 && looks_math(&runs[left - 1].text) {
            runs[left - 1].hidden = true;
            left -= 1;
        }
        let mut right = index + 1;
        while right < runs.len() && looks_math(&runs[right].text) {
            runs[right].hidden = true;
            right += 1;
        }
        index = right;
    }
}

fn emit_paragraph(out: &mut String, lines: &[&Line]) {
    let Some(first) = lines.first() else { return };
    let bold = first
        .runs
        .iter()
        .all(|run| run.style.bold && !run.text.trim().is_empty());
    let size = first.runs.iter().map(|run| run.size).max().unwrap_or(655_360);
    let heading = size > 1_310_720 || (bold && first.runs.iter().all(|run| run.text.len() < 80));
    let tag = if heading { "h2" } else { "p" };
    let _ = write!(out, "<{tag}>");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            let previous_text = lines[index - 1]
                .runs
                .last()
                .map(|run| run.text.trim_end())
                .unwrap_or("");
            if !previous_text.ends_with('-') {
                out.push(' ');
            }
        }
        let mut groups: Vec<(Style, String)> = Vec::new();
        let mut previous_end: Option<Scaled> = None;
        for run in line.runs.iter().filter(|run| !run.hidden) {
            let mut text = run.text.clone();
            if let Some(end) = previous_end
                && run.x - end > run.size / 5
            {
                text.insert(0, ' ');
            }
            match groups.last_mut() {
                Some((style, group)) if *style == run.style => group.push_str(&text),
                _ => groups.push((run.style, text)),
            }
            previous_end = Some(run.end);
        }
        for (style, text) in groups {
            let text = escape(&text);
            match (style.bold, style.italic, style.mono) {
                (true, _, _) => {
                    let _ = write!(out, "<strong>{text}</strong>");
                }
                (false, true, _) => {
                    let _ = write!(out, "<em>{text}</em>");
                }
                (false, false, true) => {
                    let _ = write!(out, "<code>{text}</code>");
                }
                (false, false, false) => out.push_str(&text),
            }
        }
    }
    let _ = writeln!(out, "</{tag}>");
}

fn fixed_page(document: &Document, page: &Page, number: usize) -> String {
    let width = pt(page.width);
    let height = pt(page.height);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{width}pt\" height=\"{height}pt\" viewBox=\"0 0 {width} {height}\">"
    );
    let _ = writeln!(out, "<title>Page {number}</title>");
    let _ = writeln!(
        out,
        "<rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"white\"/>"
    );
    for op in &page.ops {
        match op {
            DrawOp::Text {
                font,
                x,
                baseline,
                codes,
                ..
            } => {
                let docfont = document.font(*font);
                let name = docfont.map(|f| f.name.as_str()).unwrap_or("");
                let text = decode(codes, name);
                if text.trim().is_empty() {
                    continue;
                }
                let size = docfont.map_or(655_360, |f| f.size);
                let (family, weight, slant) = font_style(name);
                let _ = write!(
                    out,
                    "<text x=\"{}\" y=\"{}\" font-size=\"{}pt\" font-family=\"{family}\"",
                    pt(*x),
                    pt(*baseline),
                    pt(size)
                );
                if weight != "normal" {
                    let _ = write!(out, " font-weight=\"{weight}\"");
                }
                if slant != "normal" {
                    let _ = write!(out, " font-style=\"{slant}\"");
                }
                out.push_str(" xml:space=\"preserve\">");
                out.push_str(&escape(&text));
                out.push_str("</text>\n");
            }
            DrawOp::Rule { x, top, width, height } => {
                let _ = writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"black\"/>",
                    pt(*x),
                    pt(*top),
                    pt(*width),
                    pt(*height)
                );
            }
            _ => {}
        }
    }
    out.push_str("</svg>\n");
    out
}

fn font_style(name: &str) -> (&'static str, &'static str, &'static str) {
    let name = name.to_ascii_lowercase();
    let style = style_of(&name);
    let family = if style.mono {
        "monospace"
    } else if name.contains("cmss") || name.contains("sf") {
        "sans-serif"
    } else {
        "serif"
    };
    let weight = if style.bold { "bold" } else { "normal" };
    let slant = if style.italic { "italic" } else { "normal" };
    (family, weight, slant)
}

fn pt(v: Scaled) -> String {
    let value = f64::from(v) / 65536.0;
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::Paper;

    #[test]
    fn ot1_ligatures_and_typewriter_text_decode() {
        assert_eq!(decode(&[12, 32, 13], "cmr10"), "fi fl");
        assert_eq!(decode(&[15], "cmr10"), "ffl");
        assert_eq!(decode(&[92, 105, 110], "cmtt10"), "\\in");
        assert_eq!(decode(&[96, 65, 39], "cmr10"), "‘A’");
    }

    #[test]
    fn metadata_is_escaped() {
        assert_eq!(escape("A & B"), "A &amp; B");
        assert_eq!(escape("<x>"), "&lt;x&gt;");
    }

    #[test]
    fn a_reflow_book_has_the_ocf_entries() {
        let document = Document::new(Paper::A4.size());
        let epub = build(&document, &Options::default());
        let text = String::from_utf8_lossy(&epub);
        assert!(epub.starts_with(b"PK\x03\x04"));
        assert!(text.contains("mimetype"));
        assert!(text.contains("content.opf"));
        assert!(text.contains("nav.xhtml"));
    }

    #[test]
    fn fixed_layout_declares_pre_paginated_pages() {
        let size = Paper::A4.size();
        let mut document = Document::new(size);
        document.push_page(Page {
            width: size.width,
            height: size.height,
            ops: Vec::new(),
            attr: String::new(),
            resources: String::new(),
        });
        let options = Options {
            layout: Layout::Fixed,
            metadata: Metadata::default(),
        };
        let epub = build(&document, &options);
        let opf = entries(&epub)
            .into_iter()
            .find(|(name, _)| name == "OEBPS/content.opf")
            .map(|(_, data)| String::from_utf8_lossy(&data).into_owned())
            .unwrap();
        assert!(opf.contains("pre-paginated"));
        assert!(opf.contains("image/svg+xml"));
    }
}
