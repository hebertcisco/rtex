//! The items of TeX's lists (§133–§161), held in vectors instead of linked memory.

use std::sync::Arc;

use rtex_core::{Glue, GlueOrder, Token};

use crate::units::Scaled;

/// A dimension that a rule takes from its enclosing box (`null_flag`, §138).
pub const RUNNING: Scaled = -0o10000000000;
/// Penalties at or above this value forbid a break (`inf_penalty`).
pub const INF_PENALTY: i32 = 10_000;
/// Penalties at or below this value force a break (`eject_penalty`).
pub const EJECT_PENALTY: i32 = -10_000;
/// The badness of a box that cannot stretch enough (`inf_bad`).
pub const INF_BAD: i32 = 10_000;
/// The largest dimension TeX allows.
pub const MAX_DIMEN: Scaled = 0o7777777777;

/// Whether a box stacks its contents horizontally or vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxDir {
    /// An hbox (`hlist_node`).
    H,
    /// A vbox (`vlist_node`).
    V,
}

/// How a box's glue is set (§135).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlueSign {
    /// Glue keeps its natural size.
    #[default]
    Normal,
    /// Glue of `glue_order` stretches.
    Stretching,
    /// Glue of `glue_order` shrinks.
    Shrinking,
}

/// A packaged box (§135).
#[derive(Clone, Debug, PartialEq)]
pub struct BoxNode {
    /// Horizontal or vertical.
    pub dir: BoxDir,
    /// Width.
    pub width: Scaled,
    /// Depth.
    pub depth: Scaled,
    /// Height.
    pub height: Scaled,
    /// Raised or lowered (hbox in a vlist: moved right).
    pub shift: Scaled,
    /// Contents.
    pub list: Vec<Node>,
    /// The glue ratio.
    pub glue_set: f64,
    /// Stretching, shrinking or neither.
    pub glue_sign: GlueSign,
    /// The order of infinity being set.
    pub glue_order: GlueOrder,
    /// e-TeX's `box_lr=dlist`: a display line, never to be reversed.
    pub display: bool,
}

impl BoxNode {
    /// An empty box of the given direction (`new_null_box`, §136).
    pub fn new(dir: BoxDir) -> Self {
        BoxNode {
            dir,
            width: 0,
            depth: 0,
            height: 0,
            shift: 0,
            list: Vec::new(),
            glue_set: 0.0,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            display: false,
        }
    }
}

/// Where a glue node came from, which `\showbox` prints (§149).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlueKind {
    /// Ordinary glue.
    Normal,
    /// Glue from parameter `i` (index into `GLUE_PARAMS`).
    Param(u8),
    /// `\nonscript`.
    CondMath,
    /// `\mskip`.
    Mu,
    /// `\leaders`.
    ALeaders,
    /// `\cleaders`.
    CLeaders,
    /// `\xleaders`.
    XLeaders,
}

impl GlueKind {
    /// Whether this glue carries leaders.
    pub fn is_leaders(self) -> bool {
        matches!(self, GlueKind::ALeaders | GlueKind::CLeaders | GlueKind::XLeaders)
    }
}

/// A glue node (§149).
#[derive(Clone, Debug, PartialEq)]
pub struct GlueNode {
    /// The specification.
    pub spec: Glue,
    /// Its origin.
    pub kind: GlueKind,
    /// The box or rule repeated by leaders.
    pub leader: Option<Box<Node>>,
    /// Whether the specification is TeX's shared `zero_glue`, which `short_display` omits.
    pub zero_ptr: bool,
}

impl GlueNode {
    /// Ordinary glue.
    pub fn new(spec: Glue) -> Self {
        GlueNode {
            spec,
            kind: GlueKind::Normal,
            leader: None,
            zero_ptr: false,
        }
    }
}

/// Why a kern was inserted (§155).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernKind {
    /// From a font or implicit.
    Normal,
    /// `\kern` or italic correction.
    Explicit,
    /// Around an accent.
    Accent,
    /// `\mkern`.
    Mu,
}

/// A ligature (§143).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LigatureNode {
    /// The font.
    pub font: usize,
    /// The ligature character.
    pub ch: u32,
    /// The characters it replaces.
    pub original: Vec<u32>,
    /// Formed from a left boundary.
    pub left_hit: bool,
    /// Formed with a right boundary.
    pub right_hit: bool,
}

/// A discretionary break (§145).
#[derive(Clone, Debug, PartialEq)]
pub struct DiscNode {
    /// Before the break.
    pub pre: Vec<Node>,
    /// After the break.
    pub post: Vec<Node>,
    /// How many following items it replaces when taken.
    pub replace_count: usize,
}

/// An insertion (§140).
#[derive(Clone, Debug, PartialEq)]
pub struct InsNode {
    /// The `\insert` number.
    pub number: u8,
    /// Natural height plus depth.
    pub height: Scaled,
    /// `\splitmaxdepth`.
    pub depth: Scaled,
    /// `\splittopskip`.
    pub split_top: Glue,
    /// `\floatingpenalty`.
    pub float_cost: i32,
    /// The vertical list.
    pub list: Vec<Node>,
}

/// A mark (§141).
#[derive(Clone, Debug, PartialEq)]
pub struct MarkNode {
    /// The e-TeX mark class.
    pub class: i32,
    /// The text.
    pub toks: Arc<[Token]>,
}

/// A whatsit (§1341).
#[derive(Clone, Debug, PartialEq)]
pub enum Whatsit {
    /// `\openout`.
    Open {
        /// Stream.
        stream: u8,
        /// File name.
        name: String,
    },
    /// `\write`.
    Write {
        /// Stream; 16 is the terminal, 17 the log only.
        stream: u8,
        /// Unexpanded text.
        toks: Arc<[Token]>,
    },
    /// `\closeout`.
    Close {
        /// Stream.
        stream: u8,
    },
    /// `\special`.
    Special(Arc<[Token]>),
    /// `\setlanguage` or a language change in a paragraph.
    Language {
        /// Language number.
        lang: i32,
        /// `\lefthyphenmin`.
        lhm: i32,
        /// `\righthyphenmin`.
        rhm: i32,
    },
    /// A backend-specific item (pdfTeX's literals, links, images…).
    Backend(Arc<crate::tex::backend::BackendItem>),
}

/// Before or after math (§147).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathEdge {
    /// `$` opening.
    Before,
    /// `$` closing.
    After,
}

/// An unset box in an alignment (§159).
#[derive(Clone, Debug, PartialEq)]
pub struct UnsetNode {
    /// Width.
    pub width: Scaled,
    /// Depth.
    pub depth: Scaled,
    /// Height.
    pub height: Scaled,
    /// Columns spanned minus one.
    pub span_count: u16,
    /// Total stretch.
    pub stretch: Scaled,
    /// Total shrink.
    pub shrink: Scaled,
    /// Order of the stretch.
    pub stretch_order: GlueOrder,
    /// Order of the shrink.
    pub shrink_order: GlueOrder,
    /// Contents.
    pub list: Vec<Node>,
}

/// A field of a noad (§681): empty, a character, a box or a subformula.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum MathField {
    /// Nothing.
    #[default]
    Empty,
    /// A character of a family.
    Char {
        /// The family.
        fam: u8,
        /// The character.
        ch: u32,
    },
    /// A character whose italic correction is dubious (`math_text_char`).
    TextChar {
        /// The family.
        fam: u8,
        /// The character.
        ch: u32,
    },
    /// A box.
    SubBox(Box<BoxNode>),
    /// A subformula.
    SubMlist(Vec<Node>),
}

/// A delimiter field: small and large variants (§683).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delimiter {
    /// Family of the small variant.
    pub small_fam: u8,
    /// Character of the small variant.
    pub small_char: u32,
    /// Family of the large variant.
    pub large_fam: u8,
    /// Character of the large variant.
    pub large_char: u32,
}

impl Delimiter {
    /// Whether every field is zero (`null_delimiter`).
    pub fn is_null(&self) -> bool {
        *self == Delimiter::default()
    }
}

/// The kind of a noad (§682–§687).
#[derive(Clone, Debug, PartialEq)]
pub enum NoadKind {
    /// Ord.
    Ord,
    /// Op.
    Op,
    /// Bin.
    Bin,
    /// Rel.
    Rel,
    /// Open.
    Open,
    /// Close.
    Close,
    /// Punct.
    Punct,
    /// Inner.
    Inner,
    /// `\radical`.
    Radical(Delimiter),
    /// `\overline`.
    Over,
    /// `\underline`.
    Under,
    /// `\mathaccent`.
    Accent {
        /// Family of the accent.
        fam: u8,
        /// The accent character.
        ch: u32,
    },
    /// `\vcenter`.
    VCenter,
    /// `\left`.
    Left(Delimiter),
    /// `\right`, or e-TeX's `\middle`.
    Right {
        /// The delimiter.
        delim: Delimiter,
        /// Whether it is a `\middle`.
        middle: bool,
    },
}

/// `\limits`, `\nolimits` or the default (§682).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Limits {
    /// `\displaylimits`.
    #[default]
    Normal,
    /// `\limits`.
    Limits,
    /// `\nolimits`.
    NoLimits,
}

/// A noad (§681).
#[derive(Clone, Debug, PartialEq)]
pub struct Noad {
    /// The kind.
    pub kind: NoadKind,
    /// Limits of an Op.
    pub limits: Limits,
    /// The nucleus.
    pub nucleus: MathField,
    /// The superscript.
    pub supscr: MathField,
    /// The subscript.
    pub subscr: MathField,
    /// The translation made by the first pass of `mlist_to_hlist`.
    pub new_hlist: Option<Vec<Node>>,
}

impl Noad {
    /// An empty noad of the given kind (`new_noad`).
    pub fn new(kind: NoadKind) -> Self {
        Noad {
            kind,
            limits: Limits::Normal,
            nucleus: MathField::Empty,
            supscr: MathField::Empty,
            subscr: MathField::Empty,
            new_hlist: None,
        }
    }

    /// Whether sub- and superscripts may be attached (`scripts_allowed`).
    pub fn scripts_allowed(&self) -> bool {
        !matches!(self.kind, NoadKind::Left(_) | NoadKind::Right { .. })
    }
}

/// A generalized fraction (§683).
#[derive(Clone, Debug, PartialEq)]
pub struct FractionNoad {
    /// The rule thickness, or `None` for the default.
    pub thickness: Option<Scaled>,
    /// The numerator.
    pub numerator: MathField,
    /// The denominator.
    pub denominator: MathField,
    /// The left delimiter.
    pub left: Delimiter,
    /// The right delimiter.
    pub right: Delimiter,
    /// The translation made by the first pass.
    pub new_hlist: Option<Vec<Node>>,
}

/// An item of a horizontal or vertical list.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A character.
    Char {
        /// The font.
        font: usize,
        /// The character code.
        ch: u32,
    },
    /// An hbox or vbox.
    Box(Box<BoxNode>),
    /// A rule.
    Rule {
        /// Width or [`RUNNING`].
        width: Scaled,
        /// Depth or [`RUNNING`].
        depth: Scaled,
        /// Height or [`RUNNING`].
        height: Scaled,
    },
    /// An insertion.
    Ins(Box<InsNode>),
    /// A mark.
    Mark(MarkNode),
    /// Vertical material from `\vadjust`.
    Adjust(Vec<Node>),
    /// A ligature.
    Ligature(LigatureNode),
    /// A discretionary.
    Disc(Box<DiscNode>),
    /// A whatsit.
    Whatsit(Whatsit),
    /// Math on or off.
    Math {
        /// `\mathsurround`.
        width: Scaled,
        /// Which side.
        edge: MathEdge,
    },
    /// Glue.
    Glue(GlueNode),
    /// A kern.
    Kern {
        /// The amount.
        width: Scaled,
        /// Its origin.
        kind: KernKind,
    },
    /// A penalty.
    Penalty(i32),
    /// An unset box.
    Unset(Box<UnsetNode>),
    /// pdfTeX's margin kern, added by character protrusion at either end of a line.
    MarginKern {
        /// Its width.
        width: Scaled,
        /// At the left margin rather than the right.
        left: bool,
    },
    /// A noad, in a math list.
    Noad(Box<Noad>),
    /// A fraction noad, in a math list.
    Fraction(Box<FractionNoad>),
    /// A style change, in a math list.
    Style(u8),
    /// `\mathchoice`: the display, text, script and scriptscript lists.
    Choice(Box<[Vec<Node>; 4]>),
}

impl Node {
    /// A rule with the given dimensions.
    pub fn rule(width: Scaled, height: Scaled, depth: Scaled) -> Node {
        Node::Rule { width, depth, height }
    }

    /// A kern of the given kind.
    pub fn kern(width: Scaled, kind: KernKind) -> Node {
        Node::Kern { width, kind }
    }

    /// Ordinary glue.
    pub fn glue(spec: Glue) -> Node {
        Node::Glue(GlueNode::new(spec))
    }

    /// Whether this is a character (`is_char_node`).
    pub fn is_char(&self) -> bool {
        matches!(self, Node::Char { .. })
    }

    /// TeX's `precedes_break`: types below `math_node` (§148).
    pub fn precedes_break(&self) -> bool {
        matches!(
            self,
            Node::Char { .. }
                | Node::Box(_)
                | Node::Rule { .. }
                | Node::Ins(_)
                | Node::Mark(_)
                | Node::Adjust(_)
                | Node::Ligature(_)
                | Node::Disc(_)
                | Node::Whatsit(_)
        )
    }

    /// TeX's `non_discardable`: types below `math_node` (§148).
    pub fn non_discardable(&self) -> bool {
        self.precedes_break()
    }

    /// e-TeX's `\lastnodetype` code (-1 none, 0 char, 1 hlist, …, 15 unset).
    pub fn type_code(&self) -> i32 {
        match self {
            Node::Char { .. } => 0,
            Node::Box(b) if b.dir == BoxDir::H => 1,
            Node::Box(_) => 2,
            Node::Rule { .. } => 3,
            Node::Ins(_) => 4,
            Node::Mark(_) => 5,
            Node::Adjust(_) => 6,
            Node::Ligature(_) => 7,
            Node::Disc(_) => 8,
            Node::Whatsit(_) => 9,
            Node::Math { .. } => 10,
            Node::Glue(_) => 11,
            Node::Kern { .. } => 12,
            Node::Penalty(_) => 13,
            Node::Unset(_) => 14,
            Node::Noad(_) | Node::Fraction(_) | Node::Style(_) | Node::Choice(_) | Node::MarginKern { .. } => 15,
        }
    }
}
