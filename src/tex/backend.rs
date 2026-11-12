//! pdfTeX's whatsits and the document-wide state its primitives build: objects, forms, images,
//! outlines, colour stacks and the like.

use std::collections::HashMap;
use std::sync::Arc;

use rtex_core::{Glue, Token};

use super::Eng;
use super::node::{BoxNode, RUNNING};
use super::show::print_mark;
use crate::document::DrawOp;
use crate::images::Image;
use crate::units::Scaled;

/// How a literal is placed (`\pdfliteral direct`, `page`, or relative to the current point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralMode {
    /// The origin is moved to the current point first.
    SetOrigin,
    /// Inserted as is, outside any text object.
    Page,
    /// Inserted as is, inside a text object if one is open.
    Direct,
}

/// A `\pdfcolorstack` operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorOp {
    /// `set`.
    Set,
    /// `push`.
    Push,
    /// `pop`.
    Pop,
    /// `current`.
    Current,
}

/// A `<rule spec>` of an annotation, link, thread or `fitr` destination; [`RUNNING`] when not given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AltRule {
    /// Width.
    pub width: Scaled,
    /// Height.
    pub height: Scaled,
    /// Depth.
    pub depth: Scaled,
}

impl Default for AltRule {
    fn default() -> Self {
        AltRule {
            width: RUNNING,
            height: RUNNING,
            depth: RUNNING,
        }
    }
}

/// A destination or thread identifier: `num n` or `name{...}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Ident {
    /// `num n`.
    Num(i32),
    /// `name{...}`.
    Name(Arc<[Token]>),
}

/// The fit of a destination (`\pdfdest ... xyz`, `fit`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestKind {
    /// `xyz`, with an optional zoom.
    Xyz(Option<i32>),
    /// `fit`.
    Fit,
    /// `fith`.
    FitH,
    /// `fitv`.
    FitV,
    /// `fitb`.
    FitB,
    /// `fitbh`.
    FitBH,
    /// `fitbv`.
    FitBV,
    /// `fitr`.
    FitR,
}

/// Where an action goes (`goto`, `page`, `thread` or `user`).
#[derive(Clone, Debug, PartialEq)]
pub enum ActionTarget {
    /// `user{...}`: the action dictionary itself.
    User(Arc<[Token]>),
    /// `goto` with an identifier.
    GoTo(Ident),
    /// `goto page n {...}`.
    Page(i32, Arc<[Token]>),
    /// `thread` with an identifier.
    Thread(Ident),
}

/// An action specification (`scan_action`).
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    /// The target.
    pub target: ActionTarget,
    /// `file{...}`.
    pub file: Option<Arc<[Token]>>,
    /// `struct ...` identifier.
    pub structure: Option<Ident>,
    /// 0, 1 for `newwindow`, 2 for `nonewwindow`.
    pub new_window: u8,
}

/// A whatsit made by a pdfTeX primitive.
#[derive(Clone, Debug, PartialEq)]
pub enum BackendItem {
    /// `\pdfliteral`, or `\pdfliteral shipout` when `late`.
    Literal {
        /// Placement.
        mode: LiteralMode,
        /// Expanded at shipout rather than when scanned.
        late: bool,
        /// The PDF code.
        toks: Arc<[Token]>,
    },
    /// `\special shipout{...}`, expanded at shipout.
    LateSpecial(Arc<[Token]>),
    /// `\pdfcolorstack`.
    ColorStack {
        /// The stack number.
        stack: usize,
        /// The operation.
        op: ColorOp,
        /// The colour, for `set` and `push`.
        toks: Option<Arc<[Token]>>,
    },
    /// `\pdfsetmatrix`.
    SetMatrix(Arc<[Token]>),
    /// `\pdfsave`.
    Save,
    /// `\pdfrestore`.
    Restore,
    /// `\pdfrefobj n`.
    RefObj(usize),
    /// `\pdfrefxform n`.
    RefXForm(usize),
    /// `\pdfrefximage n`.
    RefXImage(usize),
    /// `\pdfannot`.
    Annot {
        /// The object number.
        obj: usize,
        /// The rectangle.
        rule: AltRule,
        /// The annotation dictionary contents.
        toks: Arc<[Token]>,
    },
    /// `\pdfstartlink`.
    StartLink {
        /// The object number of the link annotation.
        obj: usize,
        /// The rectangle.
        rule: AltRule,
        /// `attr{...}`.
        attr: Option<Arc<[Token]>>,
        /// The action.
        action: Arc<Action>,
    },
    /// `\pdfendlink`.
    EndLink,
    /// `\pdfdest`.
    Dest {
        /// `struct n`.
        structure: Option<i32>,
        /// The identifier.
        id: Ident,
        /// The fit.
        kind: DestKind,
        /// The rectangle of a `fitr` destination.
        rule: AltRule,
    },
    /// `\pdfthread` or `\pdfstartthread`.
    Thread {
        /// `\pdfstartthread`.
        start: bool,
        /// The rectangle.
        rule: AltRule,
        /// `attr{...}`.
        attr: Option<Arc<[Token]>>,
        /// The identifier.
        id: Ident,
    },
    /// `\pdfendthread`.
    EndThread,
    /// `\pdfsavepos`.
    SavePos,
    /// `\pdfsnaprefpoint`.
    SnapRefPoint,
    /// `\pdfsnapy`: the snap glue and the final skip.
    SnapY(Glue, Glue),
    /// `\pdfsnapycomp`.
    SnapYComp(i32),
    /// `\pdfinterwordspaceon`.
    InterwordSpaceOn,
    /// `\pdfinterwordspaceoff`.
    InterwordSpaceOff,
    /// `\pdffakespace`.
    FakeSpace,
    /// `\pdfrunninglinkoff`.
    RunningLinkOff,
    /// `\pdfrunninglinkon`.
    RunningLinkOn,
    /// Semantic structure for EPUB (`\special{rtex:...}`).
    Structure(Arc<str>),
}

fn print_rule_dimen(eng: &mut Eng, d: Scaled) {
    if d == RUNNING {
        eng.print_char(u32::from('*'));
    } else {
        eng.print_scaled(d);
    }
}

fn print_rule(eng: &mut Eng, r: &AltRule) {
    eng.print_str("(");
    print_rule_dimen(eng, r.height);
    eng.print_char(u32::from('+'));
    print_rule_dimen(eng, r.depth);
    eng.print_str(")x");
    print_rule_dimen(eng, r.width);
}

fn print_ident(eng: &mut Eng, id: &Ident) {
    match id {
        Ident::Name(t) => {
            eng.print_str(" name");
            print_mark(eng, t);
        }
        Ident::Num(n) => {
            eng.print_str(" num");
            eng.print_int(*n);
        }
    }
}

impl BackendItem {
    /// Whether this whatsit has dimensions boxes and paragraphs must count (`\pdfrefxform`, `\pdfrefximage`).
    pub(crate) fn dimensions(&self, state: &BackendState) -> Option<(Scaled, Scaled, Scaled)> {
        match self {
            BackendItem::RefXForm(n) | BackendItem::RefXImage(n) => state.object_size(*n),
            _ => None,
        }
    }

    /// Displays the item as `\showbox` does (§1356 with pdfTeX's cases).
    pub(crate) fn show(&self, eng: &mut Eng) {
        match self {
            BackendItem::Literal { mode, late, toks } => {
                eng.print_esc("pdfliteral");
                if *late {
                    eng.print_str(" shipout");
                }
                match mode {
                    LiteralMode::SetOrigin => {}
                    LiteralMode::Page => eng.print_str(" page"),
                    LiteralMode::Direct => eng.print_str(" direct"),
                }
                print_mark(eng, toks);
            }
            BackendItem::LateSpecial(toks) => {
                eng.print_esc("special");
                eng.print_str(" shipout");
                print_mark(eng, toks);
            }
            BackendItem::ColorStack { stack, op, toks } => {
                eng.print_esc("pdfcolorstack ");
                eng.print_int(*stack as i32);
                eng.print_str(match op {
                    ColorOp::Set => " set ",
                    ColorOp::Push => " push ",
                    ColorOp::Pop => " pop",
                    ColorOp::Current => " current",
                });
                if let Some(t) = toks {
                    print_mark(eng, t);
                }
            }
            BackendItem::SetMatrix(toks) => {
                eng.print_esc("pdfsetmatrix");
                print_mark(eng, toks);
            }
            BackendItem::Save => eng.print_esc("pdfsave"),
            BackendItem::Restore => eng.print_esc("pdfrestore"),
            BackendItem::RefObj(n) => {
                eng.print_esc("pdfrefobj");
                let obj = eng.stomach.backend.objects.get(n).cloned();
                if let Some(PdfObject::Raw {
                    stream,
                    stream_attr,
                    file,
                    toks,
                }) = obj
                {
                    if stream {
                        if let Some(a) = stream_attr {
                            eng.print_str(" attr");
                            print_mark(eng, &a);
                        }
                        eng.print_str(" stream");
                    }
                    if file {
                        eng.print_str(" file");
                    }
                    print_mark(eng, &toks);
                }
            }
            BackendItem::RefXForm(n) | BackendItem::RefXImage(n) => {
                eng.print_esc(if matches!(self, BackendItem::RefXForm(_)) {
                    "pdfrefxform"
                } else {
                    "pdfrefximage"
                });
                let (w, h, d) = eng.stomach.backend.object_size(*n).unwrap_or((0, 0, 0));
                eng.print_str("(");
                eng.print_scaled(h);
                eng.print_char(u32::from('+'));
                eng.print_scaled(d);
                eng.print_str(")x");
                eng.print_scaled(w);
            }
            BackendItem::Annot { rule, toks, .. } => {
                eng.print_esc("pdfannot");
                print_rule(eng, rule);
                print_mark(eng, toks);
            }
            BackendItem::StartLink { rule, attr, action, .. } => {
                eng.print_esc("pdfstartlink");
                print_rule(eng, rule);
                if let Some(a) = attr {
                    eng.print_str(" attr");
                    print_mark(eng, a);
                }
                eng.print_str(" action");
                show_action(eng, action);
            }
            BackendItem::EndLink => eng.print_esc("pdfendlink"),
            BackendItem::Dest {
                structure,
                id,
                kind,
                rule,
            } => {
                eng.print_esc("pdfdest");
                if let Some(s) = structure {
                    eng.print_str(" struct");
                    eng.print_int(*s);
                }
                print_ident(eng, id);
                eng.print_str(" ");
                match kind {
                    DestKind::Xyz(zoom) => {
                        eng.print_str("xyz");
                        if let Some(z) = zoom {
                            eng.print_str(" zoom");
                            eng.print_int(*z);
                        }
                    }
                    DestKind::FitBH => eng.print_str("fitbh"),
                    DestKind::FitBV => eng.print_str("fitbv"),
                    DestKind::FitB => eng.print_str("fitb"),
                    DestKind::FitH => eng.print_str("fith"),
                    DestKind::FitV => eng.print_str("fitv"),
                    DestKind::FitR => {
                        eng.print_str("fitr");
                        print_rule(eng, rule);
                    }
                    DestKind::Fit => eng.print_str("fit"),
                }
            }
            BackendItem::Thread { start, rule, attr, id } => {
                eng.print_esc(if *start { "pdfstartthread" } else { "pdfthread" });
                print_rule(eng, rule);
                if let Some(a) = attr {
                    eng.print_str(" attr");
                    print_mark(eng, a);
                }
                print_ident(eng, id);
            }
            BackendItem::EndThread => eng.print_esc("pdfendthread"),
            BackendItem::SavePos => eng.print_esc("pdfsavepos"),
            BackendItem::SnapRefPoint => eng.print_esc("pdfsnaprefpoint"),
            BackendItem::SnapY(snap, last) => {
                eng.print_esc("pdfsnapy");
                eng.print_char(u32::from(' '));
                eng.print_spec(snap, "");
                eng.print_char(u32::from(' '));
                eng.print_spec(last, "");
            }
            BackendItem::SnapYComp(r) => {
                eng.print_esc("pdfsnapycomp");
                eng.print_char(u32::from(' '));
                eng.print_int(*r);
            }
            BackendItem::InterwordSpaceOn => eng.print_esc("pdfinterwordspaceon"),
            BackendItem::InterwordSpaceOff => eng.print_esc("pdfinterwordspaceoff"),
            BackendItem::FakeSpace => eng.print_esc("pdffakespace"),
            BackendItem::RunningLinkOff => eng.print_esc("pdfrunninglinkoff"),
            BackendItem::RunningLinkOn => eng.print_esc("pdfrunninglinkon"),
            BackendItem::Structure(text) => {
                eng.print_esc("special");
                eng.print_str("{rtex:");
                eng.print_str(text);
                eng.print_char(u32::from('}'));
            }
        }
    }
}

fn show_action(eng: &mut Eng, a: &Action) {
    if let ActionTarget::User(t) = &a.target {
        eng.print_str(" user");
        print_mark(eng, t);
        return;
    }
    if let Some(f) = &a.file {
        eng.print_str(" file");
        print_mark(eng, f);
    }
    match &a.target {
        ActionTarget::GoTo(id) => {
            eng.print_str(" goto");
            print_ident(eng, id);
        }
        ActionTarget::Page(n, t) => {
            eng.print_str(" page");
            eng.print_int(*n);
            print_mark(eng, t);
        }
        ActionTarget::Thread(id) => {
            eng.print_str(" thread");
            print_ident(eng, id);
        }
        ActionTarget::User(_) => {}
    }
}

/// An object made by `\pdfobj`, `\pdfxform`, `\pdfximage` or reserved by `\pdfannot`.
#[derive(Clone, Debug)]
pub enum PdfObject {
    /// `\pdfobj reserveobjnum`, not yet filled.
    Reserved,
    /// `\pdfobj`: the object's text, a stream or a file to read.
    Raw {
        /// Written as a stream.
        stream: bool,
        /// `attr{...}` of a stream.
        stream_attr: Option<Arc<[Token]>>,
        /// The data names a file.
        file: bool,
        /// The data.
        toks: Arc<[Token]>,
    },
    /// `\pdfxform`: a box as a form XObject.
    XForm {
        /// Its number among forms, which names its resource (`/Fm1`).
        index: usize,
        /// The box.
        content: Box<BoxNode>,
        /// `attr{...}`.
        attr: Option<String>,
        /// `resources{...}`.
        resources: Option<String>,
        /// The page operations of the box, filled when the document is finished.
        ops: Vec<DrawOp>,
    },
    /// `\pdfximage`: an image.
    XImage {
        /// Its number among images, which names its resource (`/Im1`).
        index: usize,
        /// The image.
        image: Arc<Image>,
        /// Width.
        width: Scaled,
        /// Height.
        height: Scaled,
        /// Depth.
        depth: Scaled,
        /// `attr{...}`.
        attr: Option<String>,
    },
    /// An annotation number reserved by `\pdfannot reserveobjnum` or used by a link.
    Annot,
}

/// A colour stack (`\pdfcolorstackinit`).
#[derive(Clone, Debug)]
pub struct ColorStack {
    /// Literal mode of its operations.
    pub mode: LiteralMode,
    /// Whether the current colour is re-emitted at the start of each page.
    pub page_start: bool,
    /// The colour values, top last; the first is the initial one.
    pub values: Vec<String>,
}

/// An outline entry (`\pdfoutline`).
#[derive(Clone, Debug)]
pub struct Outline {
    /// `attr{...}`.
    pub attr: Option<String>,
    /// The action.
    pub action: Action,
    /// `count n`.
    pub count: i32,
    /// The title text.
    pub title: String,
    /// Its parent, by index.
    pub parent: Option<usize>,
}

/// Document-wide state of pdfTeX's primitives.
#[derive(Clone, Debug, Default)]
pub struct BackendState {
    /// Objects by pdfTeX object number.
    pub objects: HashMap<usize, PdfObject>,
    /// The last object number handed out.
    pub last_number: usize,
    /// Forms made so far.
    pub xform_count: usize,
    /// Images made so far.
    pub ximage_count: usize,
    /// `\pdflastobj`.
    pub last_obj: usize,
    /// `\pdflastxform`.
    pub last_xform: usize,
    /// `\pdflastximage`.
    pub last_ximage: usize,
    /// `\pdflastximagepages`.
    pub last_ximage_pages: i32,
    /// `\pdflastximagecolordepth`.
    pub last_ximage_colordepth: i32,
    /// `\pdflastannot`.
    pub last_annot: usize,
    /// `\pdflastlink`.
    pub last_link: usize,
    /// `\pdflastxpos`.
    pub last_x_pos: Scaled,
    /// `\pdflastypos`.
    pub last_y_pos: Scaled,
    /// `\pdfretval`.
    pub retval: i32,
    /// `\pdfinfo` text.
    pub info: Vec<String>,
    /// `\pdfcatalog` text.
    pub catalog: Vec<String>,
    /// `\pdfcatalog ... openaction`.
    pub open_action: Option<Action>,
    /// `\pdfnames` text.
    pub names: Vec<String>,
    /// `\pdftrailer` text.
    pub trailer: Vec<String>,
    /// `\pdftrailerid` text.
    pub trailer_id: Option<String>,
    /// Outline entries in order.
    pub outlines: Vec<Outline>,
    /// The parent that the next outline entry gets.
    pub outline_parent: Option<usize>,
    /// Colour stacks; stack 0 is the default one.
    pub color_stacks: Vec<ColorStack>,
    /// `\pdfglyphtounicode` definitions.
    pub to_unicode: HashMap<String, String>,
    /// Object numbers of pages referenced by `\pdfpageref` or shipped, by page number.
    pub page_objects: HashMap<usize, usize>,
    /// Object numbers of fonts referenced by `\pdffontobjnum`.
    pub font_objects: HashMap<usize, usize>,
    /// `\pdffontattr` text by font.
    pub font_attrs: HashMap<usize, String>,
    /// Characters forced into font subsets by `\pdfincludechars`.
    pub include_chars: HashMap<usize, Vec<u32>>,
    /// Fonts that get no built-in ToUnicode (`\pdfnobuiltintounicode`).
    pub no_builtin_tounicode: Vec<usize>,
    /// Names of destinations already defined, to warn on duplicates.
    pub dest_names: HashMap<String, ()>,
    /// The random seed.
    pub random_seed: i32,
    /// `\pdflastmatch` captures.
    pub last_match: Vec<Option<(usize, String)>>,
}

impl BackendState {
    /// The width, height and depth of a node that is a `\pdfrefxform` or `\pdfrefximage` whatsit.
    pub(crate) fn node_size(&self, n: &super::node::Node) -> Option<(Scaled, Scaled, Scaled)> {
        match n {
            super::node::Node::Whatsit(super::node::Whatsit::Backend(item)) => item.dimensions(self),
            _ => None,
        }
    }

    /// A fresh object number.
    pub fn new_number(&mut self) -> usize {
        self.last_number += 1;
        self.last_number
    }

    /// The size of a form or image object.
    pub fn object_size(&self, n: usize) -> Option<(Scaled, Scaled, Scaled)> {
        match self.objects.get(&n)? {
            PdfObject::XForm { content, .. } => Some((content.width, content.height, content.depth)),
            PdfObject::XImage {
                width, height, depth, ..
            } => Some((*width, *height, *depth)),
            _ => None,
        }
    }

    /// The object number of page `n`, allocating one if needed.
    pub fn page_object(&mut self, n: usize) -> usize {
        if let Some(&o) = self.page_objects.get(&n) {
            return o;
        }
        let o = self.new_number();
        self.page_objects.insert(n, o);
        o
    }
}
