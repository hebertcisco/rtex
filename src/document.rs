//! The typeset document: positioned pages and the fonts they use, ready for a backend.

use crate::paper::PageSize;
use crate::tex::backend::{DestKind, LiteralMode};
use crate::units::Scaled;

/// Index of a font in the engine's font table; 0 is `\nullfont`.
pub type FontId = usize;

/// A font used on some page: its TFM name and size, as `\font` loaded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocFont {
    /// The TFM file name without extension, such as `cmr10`.
    pub name: String,
    /// The size at which it is used.
    pub size: Scaled,
    /// Its design size.
    pub design_size: Scaled,
    /// The TFM checksum.
    pub checksum: u32,
    /// Metrics for a PDF font descriptor, at `size`: the height of `h`, of `H`, the depth of `y`, the x-height.
    pub metrics: FontMetrics,
}

/// The metrics pdfTeX puts in a font descriptor, taken from the TFM at the font's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontMetrics {
    /// Height of `h`.
    pub ascent: Scaled,
    /// Height of `H`.
    pub cap_height: Scaled,
    /// Depth of `y`.
    pub descent: Scaled,
    /// `\fontdimen5`.
    pub x_height: Scaled,
}

/// A drawing operation, in scaled points from the top-left corner of the page, y downwards.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    /// A run of characters in one font on one baseline.
    Text {
        /// The font.
        font: FontId,
        /// Left edge of the first character.
        x: Scaled,
        /// The baseline.
        baseline: Scaled,
        /// The character codes.
        codes: Vec<u32>,
        /// The advance width of each character, as the typesetter measured it.
        advances: Vec<Scaled>,
    },
    /// A filled rectangle.
    Rule {
        /// Left edge.
        x: Scaled,
        /// Top edge.
        top: Scaled,
        /// Width.
        width: Scaled,
        /// Height.
        height: Scaled,
    },
    /// A `\special` at the current position.
    Special {
        /// Horizontal position.
        x: Scaled,
        /// Vertical position.
        y: Scaled,
        /// The expanded text.
        text: String,
    },
    /// A PDF operation (literal, link, image…) at a position.
    Pdf {
        /// Horizontal position.
        x: Scaled,
        /// Vertical position (the baseline).
        y: Scaled,
        /// The operation.
        op: PdfOp,
    },
}

/// A rectangle in scaled points from the top-left corner of the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    /// Left edge.
    pub left: Scaled,
    /// Top edge.
    pub top: Scaled,
    /// Right edge.
    pub right: Scaled,
    /// Bottom edge.
    pub bottom: Scaled,
}

/// Where a link or outline action goes, its strings expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfTarget {
    /// `user{...}`: an action dictionary.
    User(String),
    /// `goto name{...}`.
    Name(String),
    /// `goto num n`.
    Num(i32),
    /// `goto page n {...}`.
    Page(i32, String),
    /// `thread name{...}`.
    ThreadName(String),
    /// `thread num n`.
    ThreadNum(i32),
}

/// An action, its strings expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfAction {
    /// The target.
    pub target: PdfTarget,
    /// `file{...}`.
    pub file: Option<String>,
    /// 0, 1 for `newwindow`, 2 for `nonewwindow`.
    pub new_window: u8,
}

/// The name of a destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestName {
    /// `name{...}`.
    Name(String),
    /// `num n`.
    Num(i32),
}

/// What a PDF operation does.
#[derive(Clone, Debug, PartialEq)]
pub enum PdfOp {
    /// PDF code, placed as the mode says.
    Literal {
        /// Placement.
        mode: LiteralMode,
        /// The code.
        text: String,
    },
    /// A form XObject (`\pdfrefxform`), its reference point at the position.
    Form {
        /// The object number.
        obj: usize,
        /// Depth below the reference point.
        depth: Scaled,
    },
    /// An image XObject (`\pdfrefximage`), its reference point at the position.
    Image {
        /// The object number.
        obj: usize,
        /// Width.
        width: Scaled,
        /// Height.
        height: Scaled,
        /// Depth.
        depth: Scaled,
    },
    /// An annotation (`\pdfannot`).
    Annot {
        /// The object number.
        obj: usize,
        /// The rectangle.
        area: Area,
        /// The dictionary contents.
        data: String,
    },
    /// A link annotation (`\pdfstartlink`...`\pdfendlink`).
    Link {
        /// The object number.
        obj: usize,
        /// The rectangle.
        area: Area,
        /// `attr{...}`.
        attr: Option<String>,
        /// The action.
        action: PdfAction,
    },
    /// A destination (`\pdfdest`).
    Dest {
        /// Its name.
        name: DestName,
        /// The fit.
        kind: DestKind,
        /// The area of a `fitr` destination, or the point of the others.
        area: Area,
    },
    /// Semantic structure for EPUB.
    Structure(String),
}

/// A shipped page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Media width.
    pub width: Scaled,
    /// Media height.
    pub height: Scaled,
    /// What to draw, in order.
    pub ops: Vec<DrawOp>,
    /// `\pdfpageattr` when the page was shipped.
    pub attr: String,
    /// `\pdfpageresources` when the page was shipped.
    pub resources: String,
}

/// Every shipped page with the fonts and the media size they need.
#[derive(Clone, Debug)]
pub struct Document {
    page_size: PageSize,
    fonts: Vec<Option<DocFont>>,
    pages: Vec<Page>,
}

impl Document {
    /// An empty document whose pages default to the given size.
    pub fn new(page_size: PageSize) -> Self {
        Document {
            page_size,
            fonts: Vec::new(),
            pages: Vec::new(),
        }
    }

    /// The default media size.
    pub fn page_size(&self) -> PageSize {
        self.page_size
    }

    /// The pages shipped so far.
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// Adds a shipped page.
    pub fn push_page(&mut self, page: Page) {
        self.pages.push(page);
    }

    /// The font registered under `id`.
    pub fn font(&self, id: FontId) -> Option<&DocFont> {
        self.fonts.get(id).and_then(Option::as_ref)
    }

    /// Every registered font with its id.
    pub fn fonts(&self) -> impl Iterator<Item = (FontId, &DocFont)> {
        self.fonts
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.as_ref().map(|f| (i, f)))
    }

    /// Registers `font` under `id`, the index the engine gives it.
    pub fn set_font(&mut self, id: FontId, font: DocFont) {
        if self.fonts.len() <= id {
            self.fonts.resize(id + 1, None);
        }
        self.fonts[id] = Some(font);
    }
}
