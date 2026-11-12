//! A faithful port of TeX's stomach (`tex.web` with e-TeX and pdfTeX), plugged into rtex-core.
//!
//! The mouth (rtex-core) reads and expands; this module builds lists in every
//! mode, breaks paragraphs and pages, runs output routines and ships pages
//! into a [`Document`](crate::document::Document). Section numbers (§) refer
//! to `tex.web`.

mod align;
pub mod backend;
mod build;
mod chars;
mod fonts;
mod format;
mod hyphen;
mod linebreak;
mod math;
mod md5;
mod mlist;
pub mod node;
mod pack;
mod page;
mod pdftex;
mod random;
mod regex;
mod shipout;
mod show;
mod stomach;

use std::collections::HashMap;
use std::sync::Arc;

use rtex_core::{Engine, Token};

pub use fonts::FontStore;
pub(crate) use md5::hex_digest as md5_hex;
use node::{BoxNode, Node};

use crate::document::Document;
use crate::paper::PageSize;
use crate::units::Scaled;

pub(crate) type Eng = Engine<Typesetter>;

/// `ignore_depth` (§212).
pub const IGNORE_DEPTH: Scaled = -65_536_000;

pub(crate) const VMODE: i32 = 1;
pub(crate) const HMODE: i32 = 102;
pub(crate) const MMODE: i32 = 203;
pub(crate) const TRACING_ONLINE: usize = 29;

/// A list that is not the innermost one, with the state TeX keeps in its semantic nest (§211).
#[derive(Clone, Debug)]
pub(crate) struct SavedList {
    pub mode: i32,
    pub list: Vec<Node>,
    pub mode_line: i32,
    pub prev_graf: i32,
    pub prev_depth: Scaled,
    pub space_factor: i32,
    pub clang: i32,
    pub incompleat: Option<Box<node::FractionNoad>>,
    pub delim_ptr: Option<usize>,
}

/// Where a finished box goes (TeX's `box_context`, §1071).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxContext {
    /// Appended, shifted by the amount.
    Shift(Scaled),
    /// Stored in a register.
    SetBox { register: u16, global: bool },
    /// Shipped out.
    ShipOut,
    /// Used as leaders of the given kind.
    Leaders(node::GlueKind),
}

/// `to` or `spread`, saved by `scan_spec` (§645).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spec {
    Exactly(Scaled),
    Additional(Scaled),
}

/// What a group opened by the stomach remembers (TeX's save-stack words).
#[derive(Clone, Debug)]
pub(crate) enum GroupData {
    Box {
        context: BoxContext,
        spec: Spec,
        vtop: bool,
        level: u16,
    },
    Insert {
        number: u8,
    },
    Disc {
        part: u8,
    },
    Output,
    MathField {
        target: math::MathTarget,
    },
    EqNo {
        leqno: bool,
    },
    Choice {
        part: u8,
    },
    VCenter {
        spec: Spec,
    },
}

/// The current page's state (§980–§982).
#[derive(Clone, Debug, Default)]
pub(crate) struct PageState {
    pub contents: page::PageContents,
    pub list: Vec<Node>,
    pub max_depth: Scaled,
    pub best_break: Option<usize>,
    pub least_cost: i32,
    pub best_size: Scaled,
    pub inserts: Vec<page::PageIns>,
    pub last_glue: Option<rtex_core::Glue>,
    pub last_penalty: i32,
    pub last_kern: Scaled,
    pub last_node_type: i32,
    pub output_active: bool,
}

/// The current and saved values of TeX's marks (§382), per e-TeX class.
#[derive(Clone, Debug, Default)]
pub(crate) struct Marks {
    pub classes: HashMap<i32, [Option<Arc<[Token]>>; 5]>,
}

impl Marks {
    pub fn get(&self, class: i32, code: usize) -> Option<Arc<[Token]>> {
        self.classes.get(&class).and_then(|m| m[code].clone())
    }

    pub fn set(&mut self, class: i32, code: usize, toks: Option<Arc<[Token]>>) {
        self.classes.entry(class).or_default()[code] = toks;
    }
}

/// The typesetting stomach.
pub struct Typesetter {
    pub(crate) fonts: FontStore,
    pub(crate) nest: Vec<SavedList>,
    pub(crate) cur: Vec<Node>,
    pub(crate) mode_line: i32,
    pub(crate) prev_graf: i32,
    pub(crate) clang: i32,
    pub(crate) boxes: HashMap<i32, BoxNode>,
    next_box: i32,
    pub(crate) groups: Vec<GroupData>,
    pub(crate) page: PageState,
    pub(crate) marks: Marks,
    pub(crate) disc_ptr: [Vec<Node>; 4],
    pub(crate) adjust: Option<Vec<Node>>,
    pub(crate) pack_begin_line: i32,
    pub(crate) last_badness: i32,
    pub(crate) font_in_short_display: usize,
    pub(crate) cancel_boundary: bool,
    pub(crate) hyph: hyphen::Hyphenation,
    pub(crate) document: Document,
    pub(crate) output_name: String,
    pub(crate) pdf: Option<Vec<u8>>,
    pub(crate) backend: backend::BackendState,
    pub(crate) pdf_params: pdftex::PdfParams,
    pub(crate) font_map: crate::fonts::map::FontMap,
    pub(crate) default_map_pending: bool,
    pub(crate) encodings: HashMap<String, Vec<String>>,
    pub(crate) font_codes: pdftex::FontCodes,
    pub(crate) random: random::Random,
    pub(crate) aligns: Vec<align::Alignment>,
    pub(crate) pack_totals: ([Scaled; 4], [Scaled; 4]),
    pub(crate) preamble_tokens: Vec<Token>,
    pub(crate) incompleat: Option<Box<node::FractionNoad>>,
    pub(crate) delim_ptr: Option<usize>,
    pub(crate) just_box: Option<BoxNode>,
}

impl Typesetter {
    /// A typesetter shipping pages of the given media size; `output_name` is reported in the transcript.
    pub fn new(page_size: PageSize, output_name: impl Into<String>) -> Self {
        Typesetter {
            fonts: FontStore::default(),
            nest: Vec::new(),
            cur: Vec::new(),
            mode_line: 0,
            prev_graf: 0,
            clang: 0,
            boxes: HashMap::new(),
            next_box: 1,
            groups: Vec::new(),
            page: PageState {
                max_depth: 0,
                least_cost: page::AWFUL_BAD,
                last_penalty: 0,
                last_node_type: -1,
                ..PageState::default()
            },
            marks: Marks::default(),
            disc_ptr: Default::default(),
            adjust: None,
            pack_begin_line: 0,
            last_badness: 0,
            font_in_short_display: 0,
            cancel_boundary: false,
            hyph: hyphen::Hyphenation::default(),
            document: Document::new(page_size),
            output_name: output_name.into(),
            pdf: None,
            backend: backend::BackendState::default(),
            pdf_params: pdftex::PdfParams::default(),
            font_map: crate::fonts::map::FontMap::default(),
            default_map_pending: true,
            encodings: HashMap::new(),
            font_codes: pdftex::FontCodes::default(),
            random: random::Random::default(),
            aligns: Vec::new(),
            pack_totals: ([0; 4], [0; 4]),
            preamble_tokens: Vec::new(),
            incompleat: None,
            delim_ptr: None,
            just_box: None,
        }
    }

    /// The pages shipped so far.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Takes the PDF produced when the job finished.
    pub fn take_pdf(&mut self) -> Option<Vec<u8>> {
        self.pdf.take()
    }

    pub(crate) fn store_box(&mut self, b: BoxNode) -> i32 {
        let h = self.next_box;
        self.next_box += 1;
        self.boxes.insert(h, b);
        h
    }

    /// The contribution list: the outermost vertical list (`contrib_head`).
    pub(crate) fn contrib(&mut self) -> &mut Vec<Node> {
        match self.nest.first_mut() {
            Some(outer) => &mut outer.list,
            None => &mut self.cur,
        }
    }
}

/// Defines the stomach's own primitives in a new engine.
pub fn install(eng: &mut Eng) {
    eng.define_primitive("quitvmode", rtex_core::cmd::START_PAR, 2);
    pdftex::install(eng);
}

/// Sets pdfTeX's page size to `size`, as `pdftexconfig.tex` does for A4.
pub fn set_page_size(eng: &mut Eng, size: PageSize) {
    use pdftex::dimen;
    let base = eng.stomach.pdf_params.dimen;
    for (k, v) in [(dimen::PAGE_WIDTH, size.width), (dimen::PAGE_HEIGHT, size.height)] {
        eng.assign_in_place(rtex_core::Loc::DimenPar((base + k) as u16), rtex_core::Value::Int(v));
    }
}

/// Ensures pdfTeX is in PDF-output mode and its default backend state is ready.
pub fn ensure_pdf_mode(eng: &mut Eng) {
    // Force \pdfoutput=1 so drivers/backends select the PDF path (rtex produces PDF).
    let out = eng.stomach.pdf_params.int;
    eng.assign_in_place(
        rtex_core::Loc::IntPar((out + pdftex::int::OUTPUT) as u16),
        rtex_core::Value::Int(1),
    );
    if eng.stomach.backend.color_stacks.is_empty() {
        eng.stomach.backend.color_stacks.push(backend::ColorStack {
            mode: backend::LiteralMode::Direct,
            page_start: true,
            values: vec!["0 g 0 G".to_owned()],
        });
    }
}

/// The absolute mode as TeX's `abs(mode)`.
pub(crate) fn abs_mode(eng: &Eng) -> i32 {
    eng.mode.abs()
}

/// Appends a node to the current list (`tail_append`).
pub(crate) fn tail_append(eng: &mut Eng, n: Node) {
    eng.stomach.cur.push(n);
}

/// `push_nest` (§216).
pub(crate) fn push_nest(eng: &mut Eng) {
    let line = eng.line();
    let ts = &mut eng.stomach;
    let saved = SavedList {
        mode: eng.mode,
        list: std::mem::take(&mut ts.cur),
        mode_line: ts.mode_line,
        prev_graf: ts.prev_graf,
        prev_depth: eng.aux.prev_depth,
        space_factor: eng.aux.space_factor,
        clang: ts.clang,
        incompleat: ts.incompleat.take(),
        delim_ptr: ts.delim_ptr.take(),
    };
    ts.nest.push(saved);
    ts.prev_graf = 0;
    ts.mode_line = line;
}

/// `pop_nest` (§217): returns the list that was being built.
pub(crate) fn pop_nest(eng: &mut Eng) -> Vec<Node> {
    let Some(saved) = eng.stomach.nest.pop() else {
        return std::mem::take(&mut eng.stomach.cur);
    };
    let list = std::mem::replace(&mut eng.stomach.cur, saved.list);
    eng.mode = saved.mode;
    eng.stomach.mode_line = saved.mode_line;
    eng.stomach.prev_graf = saved.prev_graf;
    eng.aux.prev_depth = saved.prev_depth;
    eng.aux.space_factor = saved.space_factor;
    eng.stomach.clang = saved.clang;
    eng.stomach.incompleat = saved.incompleat;
    eng.stomach.delim_ptr = saved.delim_ptr;
    list
}

/// `you_cant` (§1049).
pub(crate) fn you_cant(eng: &mut Eng) {
    eng.print_err("You can't use `");
    eng.print_cmd_chr(eng.cur_cmd(), eng.cur_chr());
    eng.print_str("' in ");
    eng.print_mode(eng.mode);
}

/// Prints the identifier of font `f` as `\showbox` does (§267).
pub(crate) fn print_font_id(eng: &mut Eng, f: usize) {
    let name: Vec<u32> = match eng.fonts().get(f).and_then(|i| i.id_cs) {
        Some(id) => match eng.hash().name(id) {
            rtex_core::CsName::Name(n) => n.to_vec(),
            rtex_core::CsName::Active(c) => vec![*c],
        },
        None => "nullfont".chars().map(u32::from).collect(),
    };
    eng.print_esc_chars(&name);
}
