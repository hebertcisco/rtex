//! pdfTeX's primitives on top of rtex-core: its parameters, the `\pdf...` extensions,
//! `\pdflast...` items, conversions and per-character font codes.

use std::collections::HashMap;
use std::sync::Arc;

use rtex_core::cmd::{self, chr};
use rtex_core::{Engine, Internal, Loc, Res, Token, Value};

use super::backend::{
    Action, ActionTarget, AltRule, BackendItem, ColorOp, ColorStack, DestKind, Ident, LiteralMode, Outline, PdfObject,
};
use super::node::{Node, RUNNING, Whatsit};
use super::{Eng, VMODE, abs_mode, tail_append};
use crate::images::{PageBox, read_image, scale_image};
use crate::units::Scaled;

/// pdfTeX's integer parameters, in `pdftex.web` order.
pub(crate) const INT_PARAMS: [&str; 37] = [
    "pdfoutput",
    "pdfcompresslevel",
    "pdfdecimaldigits",
    "pdfmovechars",
    "pdfimageresolution",
    "pdfpkresolution",
    "pdfuniqueresname",
    "pdfoptionalwaysusepdfpagebox",
    "pdfoptionpdfinclusionerrorlevel",
    "pdfmajorversion",
    "pdfminorversion",
    "pdfforcepagebox",
    "pdfpagebox",
    "pdfinclusionerrorlevel",
    "pdfgamma",
    "pdfimagegamma",
    "pdfimagehicolor",
    "pdfimageapplygamma",
    "pdfadjustspacing",
    "pdfprotrudechars",
    "pdftracingfonts",
    "pdfobjcompresslevel",
    "pdfadjustinterwordglue",
    "pdfprependkern",
    "pdfappendkern",
    "pdfgentounicode",
    "pdfdraftmode",
    "pdfinclusioncopyfonts",
    "pdfsuppresswarningdupdest",
    "pdfsuppresswarningdupmap",
    "pdfsuppresswarningpagegroup",
    "pdfinfoomitdate",
    "pdfsuppressptexinfo",
    "pdfomitcharset",
    "pdfomitinfodict",
    "pdfomitprocset",
    "pdfptexuseunderscore",
];

/// Indices into [`INT_PARAMS`].
pub(crate) mod int {
    pub const OUTPUT: usize = 0;
    pub const COMPRESS_LEVEL: usize = 1;
    pub const DECIMAL_DIGITS: usize = 2;
    pub const IMAGE_RESOLUTION: usize = 4;
    pub const MINOR_VERSION: usize = 10;
    pub const PAGEBOX: usize = 12;
    pub const GEN_TOUNICODE: usize = 25;
    pub const SUPPRESS_WARNING_DUP_DEST: usize = 28;
    pub const INFO_OMIT_DATE: usize = 31;
}

/// pdfTeX's dimension parameters.
pub(crate) const DIMEN_PARAMS: [&str; 13] = [
    "pdfhorigin",
    "pdfvorigin",
    "pdfpagewidth",
    "pdfpageheight",
    "pdflinkmargin",
    "pdfdestmargin",
    "pdfthreadmargin",
    "pdffirstlineheight",
    "pdflastlinedepth",
    "pdfeachlineheight",
    "pdfeachlinedepth",
    "pdfignoreddimen",
    "pdfpxdimen",
];

/// Indices into [`DIMEN_PARAMS`].
pub(crate) mod dimen {
    pub const H_ORIGIN: usize = 0;
    pub const V_ORIGIN: usize = 1;
    pub const PAGE_WIDTH: usize = 2;
    pub const PAGE_HEIGHT: usize = 3;
    pub const LINK_MARGIN: usize = 4;
    pub const DEST_MARGIN: usize = 5;
    pub const FIRST_LINE_HEIGHT: usize = 7;
    pub const LAST_LINE_DEPTH: usize = 8;
    pub const EACH_LINE_HEIGHT: usize = 9;
    pub const EACH_LINE_DEPTH: usize = 10;
    pub const IGNORED_DIMEN: usize = 11;
    pub const PX_DIMEN: usize = 12;
}

/// pdfTeX's token-list parameters.
pub(crate) const TOKS_PARAMS: [&str; 4] = ["pdfpagesattr", "pdfpageattr", "pdfpageresources", "pdfpkmode"];

/// Indices into [`TOKS_PARAMS`].
pub(crate) mod toks {
    pub const PAGES_ATTR: usize = 0;
    pub const PAGE_ATTR: usize = 1;
    pub const PAGE_RESOURCES: usize = 2;
}

/// Where the parameters of [`INT_PARAMS`], [`DIMEN_PARAMS`] and [`TOKS_PARAMS`] start in `eqtb`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PdfParams {
    pub int: usize,
    pub dimen: usize,
    pub toks: usize,
}

const EXTENSIONS: [&str; 43] = [
    "pdfliteral",
    "pdfcolorstack",
    "pdfsetmatrix",
    "pdfsave",
    "pdfrestore",
    "pdfobj",
    "pdfrefobj",
    "pdfxform",
    "pdfrefxform",
    "pdfximage",
    "pdfrefximage",
    "pdfannot",
    "pdfstartlink",
    "pdfendlink",
    "pdfoutline",
    "pdfdest",
    "pdfthread",
    "pdfstartthread",
    "pdfendthread",
    "pdfsavepos",
    "pdfsnaprefpoint",
    "pdfsnapy",
    "pdfsnapycomp",
    "pdfinfo",
    "pdfcatalog",
    "pdfnames",
    "pdfincludechars",
    "pdffontattr",
    "pdfmapfile",
    "pdfmapline",
    "pdftrailer",
    "pdftrailerid",
    "pdfresettimer",
    "pdfsetrandomseed",
    "pdffontexpand",
    "pdfglyphtounicode",
    "pdfnobuiltintounicode",
    "pdfinterwordspaceon",
    "pdfinterwordspaceoff",
    "pdffakespace",
    "pdfrunninglinkoff",
    "pdfrunninglinkon",
    "pdfspacefont",
];

const LAST_ITEMS: [&str; 11] = [
    "pdflastobj",
    "pdflastxform",
    "pdflastximage",
    "pdflastximagepages",
    "pdflastannot",
    "pdflastxpos",
    "pdflastypos",
    "pdfretval",
    "pdflastximagecolordepth",
    "pdfrandomseed",
    "pdflastlink",
];

const CONVERTS: [&str; 25] = [
    "pdftexrevision",
    "pdftexbanner",
    "pdffontname",
    "pdffontobjnum",
    "pdffontsize",
    "pdfpageref",
    "leftmarginkern",
    "rightmarginkern",
    "pdfxformname",
    "pdfescapestring",
    "pdfescapename",
    "pdfescapehex",
    "pdfunescapehex",
    "pdfcreationdate",
    "pdffilemoddate",
    "pdffilesize",
    "pdfmdfivesum",
    "pdffiledump",
    "pdfmatch",
    "pdflastmatch",
    "pdfcolorstackinit",
    "pdfuniformdeviate",
    "pdfnormaldeviate",
    "pdfinsertht",
    "pdfximagebbox",
];

/// pdfTeX's per-character font codes, with their ranges.
pub(crate) const FONT_CODES: [(&str, i32, i32); 10] = [
    ("lpcode", -1000, 1000),
    ("rpcode", -1000, 1000),
    ("efcode", 0, 1000),
    ("tagcode", i32::MIN, i32::MAX),
    ("knbscode", -1000, 1000),
    ("stbscode", -1000, 1000),
    ("shbscode", -1000, 1000),
    ("knbccode", -1000, 1000),
    ("knaccode", -1000, 1000),
    ("pdfnoligatures", 0, 0),
];

/// Indices into [`FONT_CODES`].
pub(crate) mod code {
    pub const NO_LIGATURES: usize = 9;
}

/// `\pdffontexpand` settings of a font: stretch, shrink, step and `autoexpand`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FontExpand {
    pub stretch: i32,
    pub shrink: i32,
    pub step: i32,
    pub auto: bool,
}

/// Font codes and expansion settings set by pdfTeX's font primitives.
#[derive(Clone, Debug, Default)]
pub(crate) struct FontCodes {
    pub codes: HashMap<(usize, usize, u32), i32>,
    pub expand: HashMap<usize, FontExpand>,
}

impl FontCodes {
    /// The value of code `k` for character `c` of font `f`.
    pub fn get(&self, k: usize, f: usize, c: u32) -> i32 {
        self.codes.get(&(k, f, c)).copied().unwrap_or(0)
    }
}

const PDFTEX_REVISION: &str = "29";

/// The value of pdfTeX integer parameter `k`.
pub(crate) fn pdf_int(eng: &Eng, k: usize) -> i32 {
    eng.int_par(eng.stomach.pdf_params.int + k)
}

/// The value of pdfTeX dimension parameter `k`.
pub(crate) fn pdf_dimen(eng: &Eng, k: usize) -> Scaled {
    eng.eqtb().dimen_par(eng.stomach.pdf_params.dimen + k)
}

/// The value of pdfTeX token-list parameter `k`.
pub(crate) fn pdf_toks(eng: &Eng, k: usize) -> Option<Arc<[Token]>> {
    eng.eqtb().toks_par(eng.stomach.pdf_params.toks + k).clone()
}

/// Defines pdfTeX's primitives and parameters.
pub(crate) fn install(eng: &mut Eng) {
    let mut ints = Vec::new();
    for name in INT_PARAMS {
        if name == "pdfminorversion" {
            eng.define_primitive("pdfoptionpdfminorversion", cmd::ASSIGN_INT, ints[0] as u32 + 10);
        }
        let default = match name {
            "pdfcompresslevel" => 9,
            "pdfdecimaldigits" => 3,
            "pdfimageresolution" => 72,
            "pdfmajorversion" => 1,
            "pdfminorversion" => 4,
            "pdfgamma" => 1000,
            "pdfimagegamma" => 2200,
            "pdfimagehicolor" => 1,
            _ => 0,
        };
        ints.push(eng.define_int_param(name, default));
    }
    let mut dimens = Vec::new();
    for name in DIMEN_PARAMS {
        dimens.push(eng.define_dimen_param(name));
    }
    let mut toks = Vec::new();
    for name in TOKS_PARAMS {
        toks.push(eng.define_toks_param(name));
    }
    eng.stomach.pdf_params = PdfParams {
        int: ints[0],
        dimen: dimens[0],
        toks: toks[0],
    };
    let one_inch = (473_628_672 + 50) / 100;
    for (k, v) in [
        (dimen::H_ORIGIN, one_inch),
        (dimen::V_ORIGIN, one_inch),
        (dimen::PX_DIMEN, 65_782),
    ] {
        eng.assign_in_place(Loc::DimenPar(dimens[k] as u16), Value::Int(v));
    }
    let ignored = super::IGNORE_DEPTH;
    for k in [
        dimen::IGNORED_DIMEN,
        dimen::EACH_LINE_HEIGHT,
        dimen::EACH_LINE_DEPTH,
        dimen::FIRST_LINE_HEIGHT,
        dimen::LAST_LINE_DEPTH,
    ] {
        eng.assign_in_place(Loc::DimenPar(dimens[k] as u16), Value::Int(ignored));
    }
    eng.set_px_dimen_param(dimens[dimen::PX_DIMEN]);
    for (k, name) in EXTENSIONS.iter().enumerate() {
        eng.define_primitive(name, cmd::EXTENSION, chr::STOMACH_BASE + k as u32);
    }
    for (k, name) in LAST_ITEMS.iter().enumerate() {
        eng.define_primitive(name, cmd::LAST_ITEM, chr::STOMACH_BASE + k as u32);
    }
    for (k, name) in CONVERTS.iter().enumerate() {
        eng.define_primitive(name, cmd::CONVERT, chr::STOMACH_BASE + k as u32);
    }
    for (k, (name, _, _)) in FONT_CODES.iter().enumerate() {
        eng.define_primitive(name, cmd::ASSIGN_FONT_INT, chr::STOMACH_BASE + k as u32);
    }
    eng.stomach.backend.color_stacks.push(ColorStack {
        mode: LiteralMode::Direct,
        page_start: true,
        values: vec!["0 g 0 G".to_owned()],
    });
}

/// `pdf_error`: an error message that stops the job.
pub(crate) fn pdf_error(eng: &mut Eng, t: &str, p: &str) -> Res {
    eng.normalize_selector();
    eng.print_err("pdfTeX error");
    if !t.is_empty() {
        eng.print_str(" (");
        eng.print_str(t);
        eng.print_str(")");
    }
    eng.print_str(": ");
    eng.print_str(p);
    eng.succumb()
}

/// `pdf_warning`.
pub(crate) fn pdf_warning(eng: &mut Eng, t: &str, p: &str, prepend_nl: bool, append_nl: bool) {
    if prepend_nl {
        eng.print_ln();
    }
    eng.print_str("pdfTeX warning");
    if !t.is_empty() {
        eng.print_str(" (");
        eng.print_str(t);
        eng.print_str(")");
    }
    eng.print_str(": ");
    eng.print_str(p);
    if append_nl {
        eng.print_ln();
    }
    eng.warning_issued();
}

/// `pdftex_warn`, the warnings pdfTeX's C code prints.
pub(crate) fn pdftex_warn(eng: &mut Eng, message: &str) {
    eng.print_ln();
    eng.print_ln();
    eng.print_str("pdfTeX warning: pdftex: ");
    eng.print_str(message);
    eng.print_ln();
    eng.warning_issued();
}

fn check_pdfoutput(eng: &mut Eng, name: &str, is_error: bool) -> Res {
    if pdf_int(eng, int::OUTPUT) <= 0 {
        if is_error {
            return pdf_error(eng, name, "not allowed in DVI mode (\\pdfoutput <= 0)");
        }
        pdf_warning(
            eng,
            name,
            "not allowed in DVI mode (\\pdfoutput <= 0); ignoring it",
            true,
            true,
        );
    }
    Ok(())
}

fn ext_toks(eng: &mut Eng) -> Res<Arc<[Token]>> {
    Ok(Arc::from(eng.scan_token_list(true)?))
}

fn ext_string(eng: &mut Eng) -> Res<String> {
    let t = eng.scan_token_list(true)?;
    Ok(eng.token_list_string(&t))
}

fn whatsit(eng: &mut Eng, item: BackendItem) {
    tail_append(eng, Node::Whatsit(Whatsit::Backend(Arc::new(item))));
}

fn scan_optional_space(eng: &mut Eng) -> Res {
    eng.get_x_token()?;
    if eng.cur_cmd() != cmd::SPACER {
        eng.back_input()?;
    }
    Ok(())
}

fn scan_alt_rule(eng: &mut Eng) -> Res<AltRule> {
    let mut r = AltRule::default();
    loop {
        if eng.scan_keyword("width")? {
            r.width = eng.scan_normal_dimen()?;
        } else if eng.scan_keyword("height")? {
            r.height = eng.scan_normal_dimen()?;
        } else if eng.scan_keyword("depth")? {
            r.depth = eng.scan_normal_dimen()?;
        } else {
            return Ok(r);
        }
    }
}

fn scan_ident(eng: &mut Eng, what: &str) -> Res<Option<Ident>> {
    if eng.scan_keyword("num")? {
        let v = eng.scan_int()?;
        if v <= 0 {
            pdf_error(eng, "ext1", &format!("{what} identifier must be positive"))?;
        }
        Ok(Some(Ident::Num(v)))
    } else if eng.scan_keyword("name")? {
        Ok(Some(Ident::Name(ext_toks(eng)?)))
    } else {
        Ok(None)
    }
}

/// `scan_action`.
fn scan_action(eng: &mut Eng) -> Res<Action> {
    let kind = if eng.scan_keyword("user")? {
        0
    } else if eng.scan_keyword("goto")? {
        1
    } else if eng.scan_keyword("thread")? {
        2
    } else {
        pdf_error(eng, "ext1", "action type missing")?;
        0
    };
    if kind == 0 {
        return Ok(Action {
            target: ActionTarget::User(ext_toks(eng)?),
            file: None,
            structure: None,
            new_window: 0,
        });
    }
    let file = if eng.scan_keyword("file")? {
        Some(ext_toks(eng)?)
    } else {
        None
    };
    let mut structure = None;
    if eng.scan_keyword("struct")? {
        if kind != 1 {
            pdf_error(eng, "ext1", "only GoTo action can be used with `struct'")?;
        }
        structure = if file.is_some() {
            Some(Ident::Name(ext_toks(eng)?))
        } else {
            match scan_ident(eng, "num")? {
                Some(id) => Some(id),
                None => {
                    pdf_error(eng, "ext1", "identifier type missing")?;
                    None
                }
            }
        };
    }
    let target = if eng.scan_keyword("page")? {
        if kind != 1 {
            pdf_error(eng, "ext1", "only GoTo action can be used with `page'")?;
        }
        let n = eng.scan_int()?;
        if n <= 0 {
            pdf_error(eng, "ext1", "page number must be positive")?;
        }
        ActionTarget::Page(n, ext_toks(eng)?)
    } else if eng.scan_keyword("name")? {
        let id = Ident::Name(ext_toks(eng)?);
        if kind == 1 {
            ActionTarget::GoTo(id)
        } else {
            ActionTarget::Thread(id)
        }
    } else if eng.scan_keyword("num")? {
        if kind == 1 && file.is_some() {
            pdf_error(eng, "ext1", "`goto' option cannot be used with both `file' and `num'")?;
        }
        let n = eng.scan_int()?;
        if n <= 0 {
            pdf_error(eng, "ext1", "num identifier must be positive")?;
        }
        if kind == 1 {
            ActionTarget::GoTo(Ident::Num(n))
        } else {
            ActionTarget::Thread(Ident::Num(n))
        }
    } else {
        pdf_error(eng, "ext1", "identifier type missing")?;
        ActionTarget::GoTo(Ident::Num(1))
    };
    let new_window = if eng.scan_keyword("newwindow")? {
        scan_optional_space(eng)?;
        1
    } else if eng.scan_keyword("nonewwindow")? {
        scan_optional_space(eng)?;
        2
    } else {
        0
    };
    if new_window > 0 && (!matches!(target, ActionTarget::GoTo(_) | ActionTarget::Page(..)) || file.is_none()) {
        pdf_error(
            eng,
            "ext1",
            "`newwindow'/`nonewwindow' must be used with `goto' and `file' option",
        )?;
    }
    Ok(Action {
        target,
        file,
        structure,
        new_window,
    })
}

fn check_obj(eng: &mut Eng, n: i32, ok: fn(&PdfObject) -> bool) -> Res<usize> {
    let n = usize::try_from(n).unwrap_or(0);
    if !eng.stomach.backend.objects.get(&n).is_some_and(ok) {
        pdf_error(eng, "ext1", "cannot find referenced object")?;
    }
    Ok(n)
}

fn is_raw(o: &PdfObject) -> bool {
    matches!(o, PdfObject::Raw { .. } | PdfObject::Reserved)
}

fn is_xform(o: &PdfObject) -> bool {
    matches!(o, PdfObject::XForm { .. })
}

fn is_ximage(o: &PdfObject) -> bool {
    matches!(o, PdfObject::XImage { .. })
}

/// The non-expandable pdfTeX primitives (`extension` with a stomach payload).
pub(crate) fn command(eng: &mut Eng) -> Res {
    let k = (eng.cur_chr() - chr::STOMACH_BASE) as usize;
    let name = format!("\\{}", EXTENSIONS.get(k).copied().unwrap_or("pdf"));
    match EXTENSIONS.get(k).copied().unwrap_or("") {
        "pdfliteral" => {
            check_pdfoutput(eng, &name, true)?;
            let late = eng.scan_keyword("shipout")?;
            let mode = if eng.scan_keyword("direct")? {
                LiteralMode::Direct
            } else if eng.scan_keyword("page")? {
                LiteralMode::Page
            } else {
                LiteralMode::SetOrigin
            };
            let toks = Arc::from(eng.scan_token_list(!late)?);
            whatsit(eng, BackendItem::Literal { mode, late, toks });
        }
        "pdfcolorstack" => {
            check_pdfoutput(eng, &name, true)?;
            let mut n = eng.scan_int()?;
            if n >= eng.stomach.backend.color_stacks.len() as i32 {
                eng.print_err("Unknown color stack number ");
                eng.print_int(n);
                eng.set_help(&[
                    "Allocate and initialize a color stack with \\\\pdfcolorstackinit.",
                    "I'll use default color stack 0 here.",
                    "Proceed, with fingers crossed.",
                ]);
                eng.error()?;
                n = 0;
            }
            if n < 0 {
                eng.print_err("Invalid negative color stack number");
                eng.set_help(&["I'll use default color stack 0 here.", "Proceed, with fingers crossed."]);
                eng.error()?;
                n = 0;
            }
            let op = if eng.scan_keyword("set")? {
                Some(ColorOp::Set)
            } else if eng.scan_keyword("push")? {
                Some(ColorOp::Push)
            } else if eng.scan_keyword("pop")? {
                Some(ColorOp::Pop)
            } else if eng.scan_keyword("current")? {
                Some(ColorOp::Current)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let toks = if matches!(op, ColorOp::Set | ColorOp::Push) {
                        Some(ext_toks(eng)?)
                    } else {
                        None
                    };
                    whatsit(
                        eng,
                        BackendItem::ColorStack {
                            stack: n as usize,
                            op,
                            toks,
                        },
                    );
                }
                None => {
                    eng.print_err("Color stack action is missing");
                    eng.set_help(&[
                        "The expected actions for \\pdfcolorstack:",
                        "    set, push, pop, current",
                        "I'll ignore the color stack command.",
                    ]);
                    eng.error()?;
                }
            }
        }
        "pdfsetmatrix" => {
            check_pdfoutput(eng, &name, true)?;
            let t = ext_toks(eng)?;
            whatsit(eng, BackendItem::SetMatrix(t));
        }
        "pdfsave" => {
            check_pdfoutput(eng, &name, true)?;
            whatsit(eng, BackendItem::Save);
        }
        "pdfrestore" => {
            check_pdfoutput(eng, &name, true)?;
            whatsit(eng, BackendItem::Restore);
        }
        "pdfobj" => {
            check_pdfoutput(eng, &name, true)?;
            if eng.scan_keyword("reserveobjnum")? {
                scan_optional_space(eng)?;
                let n = eng.stomach.backend.new_number();
                eng.stomach.backend.objects.insert(n, PdfObject::Reserved);
                eng.stomach.backend.last_obj = n;
            } else {
                let mut n = None;
                if eng.scan_keyword("useobjnum")? {
                    let k = eng.scan_int()?;
                    let k = usize::try_from(k).unwrap_or(0);
                    if matches!(eng.stomach.backend.objects.get(&k), Some(PdfObject::Reserved)) {
                        n = Some(k);
                    } else {
                        pdf_warning(eng, "\\pdfobj", "invalid object number being ignored", true, true);
                        eng.stomach.backend.retval = -1;
                    }
                }
                let n = match n {
                    Some(n) => n,
                    None => eng.stomach.backend.new_number(),
                };
                let stream = eng.scan_keyword("stream")?;
                let stream_attr = if stream && eng.scan_keyword("attr")? {
                    Some(ext_toks(eng)?)
                } else {
                    None
                };
                let file = eng.scan_keyword("file")?;
                let toks = ext_toks(eng)?;
                eng.stomach.backend.objects.insert(
                    n,
                    PdfObject::Raw {
                        stream,
                        stream_attr,
                        file,
                        toks,
                    },
                );
                eng.stomach.backend.last_obj = n;
            }
        }
        "pdfrefobj" => {
            check_pdfoutput(eng, &name, true)?;
            let v = eng.scan_int()?;
            let n = check_obj(eng, v, is_raw)?;
            whatsit(eng, BackendItem::RefObj(n));
        }
        "pdfxform" => {
            check_pdfoutput(eng, &name, true)?;
            let n = eng.stomach.backend.new_number();
            let attr = if eng.scan_keyword("attr")? {
                Some(ext_string(eng)?)
            } else {
                None
            };
            let resources = if eng.scan_keyword("resources")? {
                Some(ext_string(eng)?)
            } else {
                None
            };
            let r = eng.scan_register_num()? as u16;
            let Some(b) = super::build::take_box(eng, r) else {
                return pdf_error(eng, "ext1", "\\pdfxform cannot be used with a void box");
            };
            eng.stomach.backend.xform_count += 1;
            let index = eng.stomach.backend.xform_count;
            eng.stomach.backend.objects.insert(
                n,
                PdfObject::XForm {
                    index,
                    content: Box::new(b),
                    attr,
                    resources,
                    ops: Vec::new(),
                },
            );
            eng.stomach.backend.last_xform = n;
        }
        "pdfrefxform" => {
            check_pdfoutput(eng, &name, true)?;
            let v = eng.scan_int()?;
            let n = check_obj(eng, v, is_xform)?;
            whatsit(eng, BackendItem::RefXForm(n));
        }
        "pdfximage" => {
            check_pdfoutput(eng, &name, true)?;
            scan_image(eng)?;
        }
        "pdfrefximage" => {
            check_pdfoutput(eng, &name, true)?;
            let v = eng.scan_int()?;
            let n = check_obj(eng, v, is_ximage)?;
            whatsit(eng, BackendItem::RefXImage(n));
        }
        "pdfannot" => {
            check_pdfoutput(eng, &name, true)?;
            if eng.scan_keyword("reserveobjnum")? {
                let n = eng.stomach.backend.new_number();
                eng.stomach.backend.objects.insert(n, PdfObject::Annot);
                eng.stomach.backend.last_annot = n;
                scan_optional_space(eng)?;
            } else {
                let n = if eng.scan_keyword("useobjnum")? {
                    let k = eng.scan_int()?;
                    let k = usize::try_from(k).unwrap_or(0);
                    if k == 0 || k > eng.stomach.backend.last_number {
                        pdf_error(eng, "ext1", "invalid object number")?;
                    }
                    k
                } else {
                    eng.stomach.backend.new_number()
                };
                let rule = scan_alt_rule(eng)?;
                let toks = ext_toks(eng)?;
                eng.stomach.backend.objects.insert(n, PdfObject::Annot);
                whatsit(eng, BackendItem::Annot { obj: n, rule, toks });
                eng.stomach.backend.last_annot = n;
            }
        }
        "pdfstartlink" => {
            check_pdfoutput(eng, &name, true)?;
            if abs_mode(eng) == VMODE {
                pdf_error(eng, "ext1", "\\pdfstartlink cannot be used in vertical mode")?;
            }
            let n = eng.stomach.backend.new_number();
            let rule = scan_alt_rule(eng)?;
            let attr = if eng.scan_keyword("attr")? {
                Some(ext_toks(eng)?)
            } else {
                None
            };
            let action = Arc::new(scan_action(eng)?);
            eng.stomach.backend.objects.insert(n, PdfObject::Annot);
            whatsit(
                eng,
                BackendItem::StartLink {
                    obj: n,
                    rule,
                    attr,
                    action,
                },
            );
            eng.stomach.backend.last_link = n;
        }
        "pdfendlink" => {
            check_pdfoutput(eng, &name, true)?;
            if abs_mode(eng) == VMODE {
                pdf_error(eng, "ext1", "\\pdfendlink cannot be used in vertical mode")?;
            }
            whatsit(eng, BackendItem::EndLink);
        }
        "pdfoutline" => {
            check_pdfoutput(eng, &name, true)?;
            let attr = if eng.scan_keyword("attr")? {
                Some(ext_string(eng)?)
            } else {
                None
            };
            let action = scan_action(eng)?;
            let count = if eng.scan_keyword("count")? { eng.scan_int()? } else { 0 };
            let title = ext_string(eng)?;
            add_outline(eng, attr, action, count, title);
        }
        "pdfdest" => {
            check_pdfoutput(eng, &name, true)?;
            scan_dest(eng)?;
        }
        "pdfthread" | "pdfstartthread" => {
            check_pdfoutput(eng, &name, true)?;
            let start = name == "\\pdfstartthread";
            let rule = scan_alt_rule(eng)?;
            let attr = if eng.scan_keyword("attr")? {
                Some(ext_toks(eng)?)
            } else {
                None
            };
            let id = match scan_ident(eng, "num")? {
                Some(id) => id,
                None => {
                    pdf_error(eng, "ext1", "identifier type missing")?;
                    Ident::Num(1)
                }
            };
            whatsit(eng, BackendItem::Thread { start, rule, attr, id });
        }
        "pdfendthread" => {
            check_pdfoutput(eng, &name, true)?;
            whatsit(eng, BackendItem::EndThread);
        }
        "pdfsavepos" => whatsit(eng, BackendItem::SavePos),
        "pdfsnaprefpoint" => {
            check_pdfoutput(eng, &name, true)?;
            whatsit(eng, BackendItem::SnapRefPoint);
        }
        "pdfsnapy" => {
            check_pdfoutput(eng, &name, true)?;
            let g = *eng.scan_glue(chr::GLUE_VAL)?.get();
            if g.width < 0 {
                pdf_error(eng, "ext1", "negative snap glue")?;
            }
            whatsit(eng, BackendItem::SnapY(g, rtex_core::Glue::default()));
        }
        "pdfsnapycomp" => {
            check_pdfoutput(eng, &name, true)?;
            let v = eng.scan_int()?.clamp(0, 1000);
            whatsit(eng, BackendItem::SnapYComp(v));
        }
        "pdfinfo" | "pdfcatalog" | "pdftrailer" | "pdftrailerid" => {
            check_pdfoutput(eng, &name, false)?;
            let s = ext_string(eng)?;
            let on = pdf_int(eng, int::OUTPUT) > 0;
            let state = &mut eng.stomach.backend;
            if on {
                match name.as_str() {
                    "\\pdfinfo" => state.info.push(s),
                    "\\pdfcatalog" => state.catalog.push(s),
                    "\\pdftrailer" => state.trailer.push(s),
                    _ => state.trailer_id = Some(state.trailer_id.take().unwrap_or_default() + &s),
                }
            }
            if name == "\\pdfcatalog" && eng.scan_keyword("openaction")? {
                if eng.stomach.backend.open_action.is_some() {
                    pdf_error(eng, "ext1", "duplicate of openaction")?;
                } else {
                    let a = scan_action(eng)?;
                    if on {
                        eng.stomach.backend.open_action = Some(a);
                    }
                }
            }
        }
        "pdfnames" => {
            check_pdfoutput(eng, &name, true)?;
            let s = ext_string(eng)?;
            eng.stomach.backend.names.push(s);
        }
        "pdfincludechars" => {
            check_pdfoutput(eng, &name, true)?;
            let f = scan_font(eng)?;
            let s = ext_string(eng)?;
            eng.stomach
                .backend
                .include_chars
                .entry(f)
                .or_default()
                .extend(s.chars().map(u32::from));
        }
        "pdffontattr" => {
            check_pdfoutput(eng, &name, true)?;
            let f = scan_font(eng)?;
            let s = ext_string(eng)?;
            eng.stomach.backend.font_attrs.insert(f, s);
        }
        "pdfmapfile" | "pdfmapline" => {
            check_pdfoutput(eng, &name, true)?;
            let s = ext_string(eng)?;
            map_request(eng, &s, name == "\\pdfmapfile");
        }
        "pdfresettimer" => {}
        "pdfsetrandomseed" => {
            let v = eng.scan_int()?;
            eng.stomach.backend.random_seed = v.wrapping_abs();
            eng.stomach.random = super::random::Random::new(eng.stomach.backend.random_seed);
        }
        "pdffontexpand" => {
            let f = eng.scan_font_ident()? as usize;
            eng.scan_optional_equals()?;
            let stretch = eng.scan_int()?.clamp(0, 1000);
            let shrink = eng.scan_int()?.clamp(0, 500);
            let step = eng.scan_int()?.clamp(0, 100);
            if step == 0 {
                pdf_error(eng, "font expansion", "invalid step")?;
            }
            let stretch = stretch - stretch % step;
            let shrink = shrink - shrink % step;
            if stretch == 0 && shrink == 0 {
                pdf_error(eng, "font expansion", "invalid limit(s)")?;
            }
            let auto = eng.scan_keyword("autoexpand")?;
            eng.scan_keyword("selected")?;
            eng.stomach.font_codes.expand.insert(
                f,
                FontExpand {
                    stretch,
                    shrink,
                    step,
                    auto,
                },
            );
        }
        "pdfglyphtounicode" => {
            let glyph = ext_string(eng)?;
            let unicode = ext_string(eng)?;
            eng.stomach.backend.to_unicode.insert(glyph, unicode);
        }
        "pdfnobuiltintounicode" => {
            check_pdfoutput(eng, &name, true)?;
            let f = scan_font(eng)?;
            eng.stomach.backend.no_builtin_tounicode.push(f);
        }
        "pdfinterwordspaceon" | "pdfinterwordspaceoff" | "pdffakespace" | "pdfrunninglinkoff" | "pdfrunninglinkon" => {
            check_pdfoutput(eng, &name, true)?;
            let item = match name.as_str() {
                "\\pdfinterwordspaceon" => BackendItem::InterwordSpaceOn,
                "\\pdfinterwordspaceoff" => BackendItem::InterwordSpaceOff,
                "\\pdffakespace" => BackendItem::FakeSpace,
                "\\pdfrunninglinkoff" => BackendItem::RunningLinkOff,
                _ => BackendItem::RunningLinkOn,
            };
            whatsit(eng, item);
        }
        "pdfspacefont" => {
            check_pdfoutput(eng, &name, true)?;
            ext_string(eng)?;
        }
        _ => return eng.report_illegal_case(),
    }
    Ok(())
}

/// Reads `pdftex.map` if no request replaced it yet, logging its name as pdfTeX does.
pub(crate) fn load_default_map(eng: &mut Eng) {
    if !std::mem::take(&mut eng.stomach.default_map_pending) {
        return;
    }
    if let Some(f) = eng.file_system_mut().find("pdftex.map") {
        eng.print_char(u32::from('{'));
        eng.print_str(&f.display_name);
        let duplicates = eng.stomach.font_map.apply('+', &String::from_utf8_lossy(&f.data));
        eng.print_char(u32::from('}'));
        warn_duplicates(eng, &duplicates);
    }
}

fn warn_duplicates(eng: &mut Eng, names: &[String]) {
    for n in names {
        pdftex_warn(
            eng,
            &format!("fontmap entry for `{n}' already exists, duplicates ignored"),
        );
    }
}

/// `\pdfmapfile` and `\pdfmapline` (`process_map_item`).
fn map_request(eng: &mut Eng, text: &str, is_file: bool) {
    let text = text.strip_prefix(' ').unwrap_or(text);
    let (op, rest) = match text.chars().next() {
        Some(c @ ('+' | '=' | '-')) => (c, &text[1..]),
        _ => {
            eng.stomach.default_map_pending = false;
            ('+', text)
        }
    };
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let item = if is_file {
        rest.split(' ').next().unwrap_or("")
    } else {
        rest
    };
    if item.is_empty() {
        return;
    }
    load_default_map(eng);
    let duplicates = if is_file {
        let Some(f) = eng.file_system_mut().find(item) else {
            pdftex_warn(eng, &format!("cannot open font map file ({item})"));
            return;
        };
        eng.print_char(u32::from('{'));
        eng.print_str(&f.display_name);
        let d = eng.stomach.font_map.apply(op, &String::from_utf8_lossy(&f.data));
        eng.print_char(u32::from('}'));
        d
    } else {
        eng.stomach.font_map.apply(op, item)
    };
    warn_duplicates(eng, &duplicates);
}

/// Prepares a font for PDF output when shipout first uses it (`pdf_init_font`): reads the map
/// and the font's encoding, logging their names.
pub(crate) fn init_font(eng: &mut Eng, tfm: &str) {
    if pdf_int(eng, int::OUTPUT) <= 0 {
        return;
    }
    load_default_map(eng);
    let Some(enc) = eng.stomach.font_map.get(tfm).and_then(|e| e.encoding.clone()) else {
        return;
    };
    if eng.stomach.encodings.contains_key(&enc) {
        return;
    }
    if let Some(f) = eng.file_system_mut().find(&enc) {
        eng.print_char(u32::from('{'));
        eng.print_str(&f.display_name);
        eng.print_char(u32::from('}'));
        if let Some(v) = crate::fonts::map::parse_encoding(&String::from_utf8_lossy(&f.data)) {
            eng.stomach.encodings.insert(enc, v);
        }
    }
}

fn scan_font(eng: &mut Eng) -> Res<usize> {
    let f = eng.scan_font_ident()? as usize;
    if f == 0 {
        pdf_error(eng, "font", "invalid font identifier")?;
    }
    Ok(f)
}

/// `\pdfximage` (`scan_image`).
fn scan_image(eng: &mut Eng) -> Res {
    let n = eng.stomach.backend.new_number();
    let rule = scan_alt_rule(eng)?;
    let attr = if eng.scan_keyword("attr")? {
        Some(ext_string(eng)?)
    } else {
        None
    };
    let mut page = 1;
    if eng.scan_keyword("named")? {
        ext_string(eng)?;
    } else if eng.scan_keyword("page")? {
        page = eng.scan_int()?;
    }
    if eng.scan_keyword("colorspace")? {
        eng.scan_int()?;
    }
    let mut pagebox = scan_pdf_box_spec(eng)?;
    if pagebox.is_none() {
        pagebox = box_from_code(pdf_int(eng, int::PAGEBOX));
    }
    let file = ext_string(eng)?;
    let data = eng.file_system_mut().find(&file).map(|f| f.data);
    let Some(data) = data else {
        return pdf_error(eng, "", &format!("cannot find image file '{file}'"));
    };
    let image = match read_image(&data, page, pagebox.unwrap_or(PageBox::Crop)) {
        Ok(i) => i,
        Err(e) => return pdf_error(eng, "", &format!("{e} '{file}'")),
    };
    let (w, h, d) = scale_image(
        &image,
        rule.width,
        rule.height,
        rule.depth,
        pdf_int(eng, int::IMAGE_RESOLUTION),
        RUNNING,
    );
    let state = &mut eng.stomach.backend;
    state.last_ximage_pages = image.pages;
    state.last_ximage_colordepth = image.color_depth;
    state.ximage_count += 1;
    let index = state.ximage_count;
    state.objects.insert(
        n,
        PdfObject::XImage {
            index,
            image: Arc::new(image),
            width: w,
            height: h,
            depth: d,
            attr,
        },
    );
    state.last_ximage = n;
    Ok(())
}

fn box_from_code(c: i32) -> Option<PageBox> {
    match c {
        1 => Some(PageBox::Media),
        2 => Some(PageBox::Crop),
        3 => Some(PageBox::Bleed),
        4 => Some(PageBox::Trim),
        5 => Some(PageBox::Art),
        _ => None,
    }
}

fn scan_pdf_box_spec(eng: &mut Eng) -> Res<Option<PageBox>> {
    for (k, b) in [
        ("mediabox", PageBox::Media),
        ("cropbox", PageBox::Crop),
        ("bleedbox", PageBox::Bleed),
        ("trimbox", PageBox::Trim),
        ("artbox", PageBox::Art),
    ] {
        if eng.scan_keyword(k)? {
            return Ok(Some(b));
        }
    }
    Ok(None)
}

/// `\pdfdest`.
fn scan_dest(eng: &mut Eng) -> Res {
    let structure = if eng.scan_keyword("struct")? {
        let v = eng.scan_int()?;
        if v <= 0 {
            pdf_error(eng, "ext1", "struct identifier must be positive")?;
        }
        Some(v)
    } else {
        None
    };
    let id = match scan_ident(eng, "num")? {
        Some(id) => id,
        None => {
            pdf_error(eng, "ext1", "identifier type missing")?;
            Ident::Num(1)
        }
    };
    let kind = if eng.scan_keyword("xyz")? {
        DestKind::Xyz(if eng.scan_keyword("zoom")? {
            Some(eng.scan_int()?)
        } else {
            None
        })
    } else if eng.scan_keyword("fitbh")? {
        DestKind::FitBH
    } else if eng.scan_keyword("fitbv")? {
        DestKind::FitBV
    } else if eng.scan_keyword("fitb")? {
        DestKind::FitB
    } else if eng.scan_keyword("fith")? {
        DestKind::FitH
    } else if eng.scan_keyword("fitv")? {
        DestKind::FitV
    } else if eng.scan_keyword("fitr")? {
        DestKind::FitR
    } else if eng.scan_keyword("fit")? {
        DestKind::Fit
    } else {
        pdf_error(eng, "ext1", "destination type missing")?;
        DestKind::Fit
    };
    scan_optional_space(eng)?;
    let rule = if kind == DestKind::FitR {
        scan_alt_rule(eng)?
    } else {
        AltRule::default()
    };
    if dest_already_shipped(eng, &id) {
        return Ok(());
    }
    whatsit(
        eng,
        BackendItem::Dest {
            structure,
            id,
            kind,
            rule,
        },
    );
    Ok(())
}

fn dest_key(eng: &mut Eng, id: &Ident) -> String {
    match id {
        Ident::Num(n) => format!("num{n}"),
        Ident::Name(t) => format!("name{}", eng.token_list_string(t)),
    }
}

/// `warn_dest_dup`.
fn warn_dest_dup(eng: &mut Eng, id: &Ident) {
    if pdf_int(eng, int::SUPPRESS_WARNING_DUP_DEST) > 0 {
        return;
    }
    pdf_warning(eng, "ext4", "destination with the same identifier (", true, false);
    match id {
        Ident::Name(t) => {
            eng.print_str("name");
            super::show::print_mark(eng, t);
        }
        Ident::Num(n) => {
            eng.print_str("num");
            eng.print_int(*n);
        }
    }
    eng.print_str(") has been already used, duplicate ignored");
    eng.print_ln();
    eng.show_context();
}

/// Whether a destination with this identifier was shipped out already; warns if so.
pub(crate) fn dest_already_shipped(eng: &mut Eng, id: &Ident) -> bool {
    let key = dest_key(eng, id);
    if eng.stomach.backend.dest_names.contains_key(&key) {
        warn_dest_dup(eng, id);
        return true;
    }
    false
}

/// Records a destination being shipped out; `false` when it duplicates an earlier one.
pub(crate) fn ship_dest(eng: &mut Eng, id: &Ident) -> bool {
    if dest_already_shipped(eng, id) {
        return false;
    }
    let key = dest_key(eng, id);
    eng.stomach.backend.dest_names.insert(key, ());
    true
}

/// Adds an outline entry, keeping pdfTeX's notion of the current parent (`\pdfoutline`).
fn add_outline(eng: &mut Eng, attr: Option<String>, action: Action, count: i32, title: String) {
    let state = &mut eng.stomach.backend;
    let parent = state.outline_parent;
    state.outlines.push(Outline {
        attr,
        action,
        count,
        title,
        parent,
    });
    let k = state.outlines.len() - 1;
    if count != 0 {
        state.outline_parent = Some(k);
        return;
    }
    let mut child = k;
    while let Some(p) = state.outline_parent {
        let siblings = state
            .outlines
            .iter()
            .filter(|o| o.parent == state.outlines[child].parent)
            .count();
        if siblings as i32 != state.outlines[p].count.abs() {
            break;
        }
        child = p;
        state.outline_parent = state.outlines[p].parent;
    }
}

/// `\pdflast...` and the other internal quantities of pdfTeX.
pub(crate) fn internal(eng: &mut Eng, m: u32) -> Res<Option<Internal>> {
    let k = (m - chr::STOMACH_BASE) as usize;
    let s = &eng.stomach.backend;
    Ok(Some(match LAST_ITEMS.get(k).copied().unwrap_or("") {
        "pdflastobj" => Internal::Int(s.last_obj as i32),
        "pdflastxform" => Internal::Int(s.last_xform as i32),
        "pdflastximage" => Internal::Int(s.last_ximage as i32),
        "pdflastximagepages" => Internal::Int(s.last_ximage_pages),
        "pdflastannot" => Internal::Int(s.last_annot as i32),
        "pdflastxpos" => Internal::Int(s.last_x_pos),
        "pdflastypos" => Internal::Int(s.last_y_pos),
        "pdfretval" => Internal::Int(s.retval),
        "pdflastximagecolordepth" => Internal::Int(s.last_ximage_colordepth),
        "pdfrandomseed" => Internal::Int(s.random_seed),
        "pdflastlink" => Internal::Int(s.last_link as i32),
        _ => return Ok(None),
    }))
}

/// `\lpcode` and the other font codes as internal integers.
pub(crate) fn font_code_internal(eng: &mut Eng, m: u32) -> Res<Option<Internal>> {
    let k = (m - chr::STOMACH_BASE) as usize;
    let f = eng.scan_font_ident()? as usize;
    if k == code::NO_LIGATURES {
        return Ok(Some(Internal::Int(i32::from(
            eng.stomach.font_codes.codes.contains_key(&(k, f, 0)),
        ))));
    }
    let c = eng.scan_char_num()? as u32;
    Ok(Some(Internal::Int(eng.stomach.font_codes.get(k, f, c))))
}

/// Assigns a font code (`\lpcode\font`a=50`).
pub(crate) fn assign_font_code(eng: &mut Eng, m: u32) -> Res {
    let k = (m - chr::STOMACH_BASE) as usize;
    let f = eng.scan_font_ident()? as usize;
    if k == code::NO_LIGATURES {
        eng.stomach.font_codes.codes.insert((k, f, 0), 1);
        return Ok(());
    }
    let c = eng.scan_char_num()? as u32;
    eng.scan_optional_equals()?;
    let v = eng.scan_int()?;
    let (_, lo, hi) = FONT_CODES[k];
    eng.stomach.font_codes.codes.insert((k, f, c), v.clamp(lo, hi));
    Ok(())
}

fn insert_string(eng: &mut Eng, s: &str) -> Res {
    let chars: Vec<u32> = s.chars().map(u32::from).collect();
    let toks = Engine::<super::Typesetter>::string_tokens(&chars);
    eng.ins_list(toks)
}

fn scaled_pt(eng: &mut Eng, v: Scaled) -> String {
    let mut s = String::new();
    rtex_core::format_scaled(&mut s, v);
    let _ = eng;
    s + "pt"
}

/// The expandable pdfTeX primitives (`convert` with a stomach payload).
pub(crate) fn expand(eng: &mut Eng, m: u32) -> Res {
    let k = (m - chr::STOMACH_BASE) as usize;
    let result = match CONVERTS.get(k).copied().unwrap_or("") {
        "pdftexrevision" => PDFTEX_REVISION.to_owned(),
        "pdftexbanner" => eng.config().banner.clone(),
        "pdffontname" | "pdffontobjnum" => {
            let objnum = CONVERTS[k] == "pdffontobjnum";
            let f = scan_font(eng)?;
            let state = &mut eng.stomach.backend;
            let n = match state.font_objects.get(&f) {
                Some(&n) => n,
                None => {
                    let n = state.new_number();
                    state.font_objects.insert(f, n);
                    n
                }
            };
            if objnum { n.to_string() } else { f.to_string() }
        }
        "pdffontsize" => {
            let f = scan_font(eng)?;
            let size = eng.fonts().get(f).map_or(0, |i| i.size);
            scaled_pt(eng, size)
        }
        "pdfpageref" => {
            let n = eng.scan_int()?;
            if n <= 0 {
                pdf_error(eng, "pageref", "invalid page number")?;
            }
            eng.stomach.backend.page_object(n as usize).to_string()
        }
        "leftmarginkern" | "rightmarginkern" => {
            let left = CONVERTS[k] == "leftmarginkern";
            let r = eng.scan_register_num()? as u16;
            let Some(b) = super::build::fetch_box(eng, r)
                .filter(|b| b.dir == super::node::BoxDir::H)
                .cloned()
            else {
                pdf_error(eng, "marginkern", "a non-empty hbox expected")?;
                return Ok(());
            };
            let w = super::linebreak::margin_kern_of(&eng.stomach.backend, &b, left);
            scaled_pt(eng, w)
        }
        "pdfxformname" => {
            let v = eng.scan_int()?;
            let n = check_obj(eng, v, is_xform)?;
            match eng.stomach.backend.objects.get(&n) {
                Some(PdfObject::XForm { index, .. }) => index.to_string(),
                _ => String::new(),
            }
        }
        "pdfescapestring" => {
            let s = ext_string(eng)?;
            escape_string(&s)
        }
        "pdfescapename" => {
            let s = ext_string(eng)?;
            escape_name(&s)
        }
        "pdfescapehex" => {
            let s = ext_string(eng)?;
            s.chars().map(|c| format!("{:02X}", u32::from(c) & 0xFF)).collect()
        }
        "pdfunescapehex" => {
            let s = ext_string(eng)?;
            unescape_hex(&s)
        }
        "pdfcreationdate" => creation_date(),
        "pdffilemoddate" => {
            let s = ext_string(eng)?;
            file_mod_date(eng, &s)
        }
        "pdffilesize" => {
            let s = ext_string(eng)?;
            eng.file_system_mut()
                .find(&s)
                .map(|f| f.data.len().to_string())
                .unwrap_or_default()
        }
        "pdfmdfivesum" => {
            let file = eng.scan_keyword("file")?;
            let s = ext_string(eng)?;
            let data: Option<Vec<u8>> = if file {
                eng.file_system_mut().find(&s).map(|f| f.data.to_vec())
            } else {
                Some(s.chars().map(|c| u32::from(c) as u8).collect())
            };
            data.map(|d| super::md5::hex_digest(&d)).unwrap_or_default()
        }
        "pdffiledump" => {
            let mut offset = 0;
            if eng.scan_keyword("offset")? {
                offset = eng.scan_int()?;
                if offset < 0 {
                    eng.print_err("Bad file offset");
                    eng.set_help(&[
                        "A file offset must be between 0 and 2^{31}-1,",
                        "I changed this one to zero.",
                    ]);
                    eng.int_error(offset)?;
                    offset = 0;
                }
            }
            let mut length = 0;
            if eng.scan_keyword("length")? {
                length = eng.scan_int()?;
                if length < 0 {
                    eng.print_err("Bad dump length");
                    eng.set_help(&[
                        "A dump length must be between 0 and 2^{31}-1,",
                        "I changed this one to zero.",
                    ]);
                    eng.int_error(length)?;
                    length = 0;
                }
            }
            let s = ext_string(eng)?;
            eng.file_system_mut()
                .find(&s)
                .map(|f| {
                    let start = (offset as usize).min(f.data.len());
                    let end = (start + length as usize).min(f.data.len());
                    f.data[start..end].iter().map(|b| format!("{b:02X}")).collect()
                })
                .unwrap_or_default()
        }
        "pdfmatch" => {
            let icase = eng.scan_keyword("icase")?;
            let subcount = if eng.scan_keyword("subcount")? {
                eng.scan_int()?
            } else {
                -1
            };
            let pattern = ext_string(eng)?;
            let subject = ext_string(eng)?;
            match super::regex::match_strings(&pattern, &subject, icase, subcount) {
                Ok((r, captures)) => {
                    eng.stomach.backend.last_match = captures;
                    r.to_string()
                }
                Err(e) => {
                    pdftex_warn(eng, &format!("\\pdfmatch: {}", e.message()));
                    "-1".to_owned()
                }
            }
        }
        "pdflastmatch" => {
            let mut n = eng.scan_int()?;
            if n < 0 {
                eng.print_err("Bad match number");
                eng.set_help(&[
                    "Since I expected zero or a positive number,",
                    "I changed this one to zero.",
                ]);
                eng.int_error(n)?;
                n = 0;
            }
            match eng.stomach.backend.last_match.get(n as usize) {
                Some(Some((pos, text))) => format!("{pos}->{text}"),
                _ => "-1->".to_owned(),
            }
        }
        "pdfcolorstackinit" => {
            let page_start = eng.scan_keyword("page")?;
            let mode = if eng.scan_keyword("direct")? {
                LiteralMode::Direct
            } else if eng.scan_keyword("page")? {
                LiteralMode::Page
            } else {
                LiteralMode::SetOrigin
            };
            let s = ext_string(eng)?;
            let stacks = &mut eng.stomach.backend.color_stacks;
            if stacks.len() >= 32768 {
                eng.print_err("Too many color stacks");
                eng.set_help(&[
                    "The number of color stacks is limited to 32768.",
                    "I'll use the default color stack 0 here.",
                ]);
                eng.error()?;
                "0".to_owned()
            } else {
                stacks.push(ColorStack {
                    mode,
                    page_start,
                    values: vec![s],
                });
                (stacks.len() - 1).to_string()
            }
        }
        "pdfuniformdeviate" => {
            let n = eng.scan_int()?;
            eng.stomach.random.unif_rand(n).to_string()
        }
        "pdfnormaldeviate" => eng.stomach.random.norm_rand().to_string(),
        "pdfinsertht" => {
            let n = eng.scan_register_num()?;
            let h = super::page::insert_height(eng, n);
            scaled_pt(eng, h)
        }
        "pdfximagebbox" => {
            let v = eng.scan_int()?;
            let n = check_obj(eng, v, is_ximage)?;
            let j = eng.scan_int()?;
            if !(1..=4).contains(&j) {
                pdf_error(eng, "pdfximagebbox", "invalid parameter")?;
            }
            let v = match eng.stomach.backend.objects.get(&n) {
                Some(PdfObject::XImage { image, .. }) if image.kind == crate::images::ImageKind::Pdf => {
                    let b = image.bbox;
                    let sp = |x: f64| (x * 65781.76).round() as Scaled;
                    match j {
                        1 => sp(b[0]),
                        2 => sp(b[1]),
                        3 => sp(b[2]),
                        _ => sp(b[3]),
                    }
                }
                _ => 0,
            };
            scaled_pt(eng, v)
        }
        _ => String::new(),
    };
    insert_string(eng, &result)
}

fn text_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        match u8::try_from(u32::from(c)) {
            Ok(b) => out.push(b),
            Err(_) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    out
}

/// What the PDF writer needs from pdfTeX's primitives, token lists expanded and forms rendered.
pub(crate) fn pdf_job(eng: &mut Eng) -> Res<crate::pdf::PdfJob> {
    use crate::pdf::{JobObject, JobOutline, PdfJob};
    let numbers: Vec<usize> = {
        let mut v: Vec<usize> = eng.stomach.backend.objects.keys().copied().collect();
        v.sort_unstable();
        v
    };
    let mut objects = std::collections::BTreeMap::new();
    for n in numbers {
        let Some(o) = eng.stomach.backend.objects.get(&n).cloned() else {
            continue;
        };
        let resolved = match o {
            PdfObject::Reserved | PdfObject::Annot => JobObject::Reserved,
            PdfObject::Raw {
                stream,
                stream_attr,
                file,
                toks,
            } => {
                let text = eng.token_list_string(&toks);
                let data = if file {
                    match eng.file_system_mut().find(&text) {
                        Some(f) => f.data.to_vec(),
                        None => {
                            pdf_error(eng, "ext5", &format!("cannot open file for embedding '{text}'"))?;
                            Vec::new()
                        }
                    }
                } else {
                    text_bytes(&text)
                };
                let attr = stream_attr.map(|a| eng.token_list_string(&a)).unwrap_or_default();
                JobObject::Raw { stream, attr, data }
            }
            PdfObject::XForm {
                index,
                content,
                attr,
                resources,
                ..
            } => {
                let ops = super::shipout::render_form(eng, &content)?;
                JobObject::Form {
                    index,
                    width: content.width,
                    height: content.height,
                    depth: content.depth,
                    attr: attr.unwrap_or_default(),
                    resources: resources.unwrap_or_default(),
                    ops,
                }
            }
            PdfObject::XImage { index, image, attr, .. } => JobObject::Image {
                index,
                image,
                attr: attr.unwrap_or_default(),
            },
        };
        objects.insert(n, resolved);
    }
    let outlines_src = eng.stomach.backend.outlines.clone();
    let outlines = outlines_src
        .iter()
        .map(|o| JobOutline {
            attr: o.attr.clone(),
            action: super::shipout::resolve_action(eng, &o.action),
            count: o.count,
            title: o.title.clone(),
            parent: o.parent,
        })
        .collect();
    let open_action = eng
        .stomach
        .backend
        .open_action
        .clone()
        .map(|a| super::shipout::resolve_action(eng, &a));
    let pages_attr = match pdf_toks(eng, toks::PAGES_ATTR) {
        Some(t) => eng.token_list_string(&t),
        None => String::new(),
    };
    let s = &eng.stomach.backend;
    Ok(PdfJob {
        objects,
        last_number: s.last_number,
        page_objects: s.page_objects.clone(),
        font_objects: s.font_objects.clone(),
        info: s.info.clone(),
        catalog: s.catalog.clone(),
        open_action,
        names: s.names.clone(),
        trailer: s.trailer.clone(),
        trailer_id: s.trailer_id.clone(),
        outlines,
        pages_attr,
        compress_level: pdf_int(eng, int::COMPRESS_LEVEL),
        decimal_digits: pdf_int(eng, int::DECIMAL_DIGITS),
        minor_version: pdf_int(eng, int::MINOR_VERSION),
        gen_tounicode: pdf_int(eng, int::GEN_TOUNICODE) > 0,
        info_omit_date: pdf_int(eng, int::INFO_OMIT_DATE) > 0,
        omit_charset: pdf_int(eng, 33) > 0,
        omit_info: pdf_int(eng, 34) > 0,
        to_unicode: s.to_unicode.clone(),
        font_attrs: s.font_attrs.clone(),
        date: creation_date(),
    })
}

/// `escapestring`: the characters PDF strings need escaped, with backslashes and octal codes.
pub(crate) fn escape_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let b = u32::from(c) & 0xFF;
        match b {
            0x28 | 0x29 | 0x5C => {
                out.push('\\');
                out.push(char::from(b as u8));
            }
            0x20..=0x7E => out.push(char::from(b as u8)),
            _ => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out
}

/// `escapename`: PDF name characters, with `#xx` for the rest.
pub(crate) fn escape_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let b = u32::from(c) & 0xFF;
        let delimiter = b"()<>[]{}/%#".contains(&(b as u8));
        if b == 0 {
            continue;
        }
        if !(0x21..=0x7E).contains(&b) || delimiter {
            out.push_str(&format!("#{b:02X}"));
        } else {
            out.push(char::from(b as u8));
        }
    }
    out
}

fn unescape_hex(s: &str) -> String {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(16)).collect();
    digits
        .chunks(2)
        .map(|p| {
            let v = if p.len() == 2 { p[0] * 16 + p[1] } else { p[0] * 16 };
            char::from_u32(v).unwrap_or('?')
        })
        .collect()
}

/// The creation date as pdfTeX writes it (`D:YYYYMMDDHHmmSS...`), from `SOURCE_DATE_EPOCH` when set.
pub(crate) fn creation_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        });
    format_pdf_date(secs)
}

/// Formats seconds since the epoch, in UTC, as a PDF date.
pub(crate) fn format_pdf_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn file_mod_date(eng: &mut Eng, name: &str) -> String {
    let _ = eng.file_system_mut().find(name);
    std::fs::metadata(name)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format_pdf_date(d.as_secs() as i64))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_follow_pdftex() {
        assert_eq!(escape_string("a(b)\\c\u{7}"), "a\\(b\\)\\\\c\\007");
        assert_eq!(escape_name("a b/#"), "a#20b#2F#23");
        assert_eq!(unescape_hex("414a"), "AJ");
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(format_pdf_date(0), "D:19700101000000Z");
        assert_eq!(format_pdf_date(1_700_000_000), "D:20231114221320Z");
    }
}
