//! Fonts as the stomach sees them: TFM metrics indexed by the mouth's font numbers.

use rtex_core::{FontInfo, Res};

use super::Eng;
use super::node::Node;
use crate::fonts::tfm::Tfm;
use crate::units::Scaled;

const DEFAULT_HYPHEN_CHAR: usize = 46;
const DEFAULT_SKEW_CHAR: usize = 47;
const TRACING_LOST_CHARS: usize = 35;
const TRACING_ONLINE: usize = 29;

/// The provenance of a font slot, for the pdfTeX restrictions on `\pdfcopyfont`
/// (`copy_font_info`, pdftex.web).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FontKind {
    /// Loaded from a file with `\font`.
    #[default]
    Regular,
    /// Created by `\letterspacefont` (a virtual font).
    Letterspaced,
    /// Created by `\pdfcopyfont`.
    Copied,
    /// An expanded instance created by `\pdffontexpand` (`auto_expand_font`).
    Expanded,
}

/// The metrics of every loaded font; index 0 is `\nullfont`.
#[derive(Clone, Debug, Default)]
pub struct FontStore {
    fonts: Vec<Option<Tfm>>,
    names: Vec<String>,
    kinds: Vec<FontKind>,
    virtuals: std::collections::HashMap<usize, super::vfont::VirtualFont>,
}

impl FontStore {
    /// The metrics of font `f`.
    pub fn get(&self, f: usize) -> Option<&Tfm> {
        self.fonts.get(f).and_then(Option::as_ref)
    }

    /// The file name font `f` was loaded from.
    pub fn name(&self, f: usize) -> &str {
        self.names.get(f).map_or("nullfont", String::as_str)
    }

    pub(crate) fn insert(&mut self, f: usize, tfm: Tfm, name: String) {
        self.insert_kind(f, tfm, name, FontKind::Regular);
    }

    pub(crate) fn insert_kind(&mut self, f: usize, tfm: Tfm, name: String, kind: FontKind) {
        if self.fonts.len() <= f {
            self.fonts.resize(f + 1, None);
            self.names.resize(f + 1, String::new());
            self.kinds.resize(f + 1, FontKind::Regular);
        }
        self.fonts[f] = Some(tfm);
        self.names[f] = name;
        self.kinds[f] = kind;
    }

    /// The provenance of font `f`.
    pub(crate) fn kind(&self, f: usize) -> FontKind {
        self.kinds.get(f).copied().unwrap_or_default()
    }

    /// The virtual-font program of `f`, if any.
    pub(crate) fn virtuals(&self, f: usize) -> Option<&super::vfont::VirtualFont> {
        self.virtuals.get(&f)
    }

    pub(crate) fn insert_virtual(&mut self, f: usize, vf: super::vfont::VirtualFont) {
        self.virtuals.insert(f, vf);
    }

    /// Number of slots, `\nullfont` included.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Whether no font has been loaded.
    pub fn is_empty(&self) -> bool {
        self.fonts.iter().all(Option::is_none)
    }

    /// Whether character `c` exists in font `f`.
    pub fn has_char(&self, f: usize, c: u32) -> bool {
        self.get(f).is_some_and(|t| t.has_char(c))
    }

    /// Width of `c` in `f`.
    pub fn width(&self, f: usize, c: u32) -> Scaled {
        self.get(f).map_or(0, |t| t.width(c))
    }

    /// Height of `c` in `f`.
    pub fn height(&self, f: usize, c: u32) -> Scaled {
        self.get(f).map_or(0, |t| t.height(c))
    }

    /// Depth of `c` in `f`.
    pub fn depth(&self, f: usize, c: u32) -> Scaled {
        self.get(f).map_or(0, |t| t.depth(c))
    }

    /// Italic correction of `c` in `f`.
    pub fn italic(&self, f: usize, c: u32) -> Scaled {
        self.get(f).map_or(0, |t| t.italic(c))
    }
}

/// Loads a TFM file for `\font` (`read_font_info`, §560).
pub(crate) fn load_font(eng: &mut Eng, file: &[u32], size: i32) -> Res<Option<FontInfo>> {
    let name: String = file.iter().filter_map(|&c| char::from_u32(c)).collect();
    let stem = name.strip_suffix(".tfm").unwrap_or(&name).to_owned();
    let Some(found) = eng.file_system_mut().find(&format!("{stem}.tfm")) else {
        return Ok(None);
    };
    let Ok(tfm) = Tfm::parse(&found.data, size) else {
        return Ok(None);
    };
    let f = eng.fonts().len();
    let info = FontInfo {
        name: stem.chars().map(u32::from).collect(),
        size: tfm.size,
        dsize: tfm.design_size,
        hyphen_char: eng.int_par(DEFAULT_HYPHEN_CHAR),
        skew_char: eng.int_par(DEFAULT_SKEW_CHAR),
        params: tfm.params().to_vec(),
        ..FontInfo::null()
    };
    eng.stomach.fonts.insert(f, tfm, stem);
    Ok(Some(info))
}

/// The font name pdfTeX gives an expanded instance (`expand_font_name`): `base+<e>`.
fn expand_font_name(base: &str, e: i32) -> String {
    format!("{base}{e:+}")
}

/// Creates the expanded limit instance of font `source` at expansion ratio `e`
/// (`auto_expand_font`/`get_expand_font` in pdftex.web): an anonymous font whose metrics
/// are the base ones scaled by `1000+e`/1000 and whose map lookup uses the base file
/// (`font_area[k] := font_area[f]`). Instances are cached per `(source, e)`.
pub(crate) fn expand_instance(eng: &mut Eng, source: usize, e: i32, step: i32, auto: bool) -> Res<usize> {
    if let Some(&cached) = eng.stomach.font_codes.expansions.get(&(source, e)) {
        return Ok(cached);
    }
    let Some(base_tfm) = eng.stomach.fonts.get(source).cloned() else {
        return Ok(source);
    };
    let base_info = eng.fonts().get(source).cloned();
    let Some(base_info) = base_info else {
        return Ok(source);
    };
    let base_name = eng.stomach.fonts.name(source).to_owned();
    let tfm = base_tfm.expanded(e);
    let name = expand_font_name(&base_name, e);
    let slot = eng.fonts().len();
    let info = FontInfo {
        name: name.chars().map(u32::from).collect(),
        file: base_info.file.clone(),
        size: base_info.size,
        dsize: base_info.dsize,
        hyphen_char: base_info.hyphen_char,
        skew_char: base_info.skew_char,
        params: base_info.params.clone(),
        ..FontInfo::null()
    };
    eng.stomach.fonts.insert_kind(slot, tfm, base_name, FontKind::Expanded);
    let f = eng.push_font(info);
    debug_assert_eq!(f, slot);
    eng.stomach.font_codes.expand.insert(
        f,
        super::pdftex::FontExpand {
            stretch: e.max(0),
            shrink: (-e).max(0),
            step,
            auto,
        },
    );
    eng.stomach.font_codes.expansions.insert((source, e), f);
    Ok(f)
}

/// Creates a letterspaced copy of font `source` (`\letterspacefont`).
/// `nolig` retags the copy's ligature/kern entries away (`set_no_ligatures`, pdfTeX §1354).
pub(crate) fn letterspace_font(eng: &mut Eng, source: usize, e: i32, nolig: bool) -> Res<Option<FontInfo>> {
    let Some(base_tfm) = eng.stomach.fonts.get(source).cloned() else {
        return Ok(None);
    };
    let base_info = eng.fonts().get(source).cloned();
    let Some(base_info) = base_info else {
        return Ok(None);
    };
    if base_tfm.params().get(5).copied().unwrap_or(0) == 0 {
        super::pdftex::pdf_warning(
            eng,
            "\\letterspacefont",
            "font has zero em size (\\fontdimen6)",
            true,
            true,
        );
    }
    // The virtual packets are built from the *base* font (pdfTeX: `letter_space_font`
    // reads the font fresh and attaches local font `f`), with `w = round(quad*e/2000)`.
    let quad = base_tfm.params().get(5).copied().unwrap_or(0);
    let (bc, ec) = base_tfm.char_range();
    let vf = super::vfont::VirtualFont::letterspacing(source, bc, ec, quad, e);
    let mut tfm = base_tfm.letterspaced(e);
    if nolig {
        tfm = tfm.without_lig_kern();
    }
    let base_name = eng.stomach.fonts.name(source).to_owned();
    let mut display_name = base_name.clone();
    if e >= 0 {
        display_name.push('+');
    }
    display_name.push_str(&e.to_string());
    display_name.push_str("ls");
    let f = eng.fonts().len();
    let info = FontInfo {
        name: display_name.chars().map(u32::from).collect(),
        file: base_info.file.clone(),
        size: base_info.size,
        dsize: base_info.dsize,
        hyphen_char: base_info.hyphen_char,
        skew_char: base_info.skew_char,
        params: tfm.params().to_vec(),
        ..FontInfo::null()
    };
    // Store base name for PDF/font-file lookup; \fontname uses core FontInfo.name.
    eng.stomach.fonts.insert_kind(f, tfm, base_name, FontKind::Letterspaced);
    eng.stomach.fonts.insert_virtual(f, vf);
    Ok(Some(info))
}

/// Creates a copy of font `source` (`\pdfcopyfont`, pdftex.web `copy_font_info`).
/// Refuses to copy letterspaced fonts and fonts with expansion parameters, as pdfTeX does.
pub(crate) fn copy_font(eng: &mut Eng, source: usize) -> Res<Option<FontInfo>> {
    if matches!(
        eng.stomach.fonts.kind(source),
        FontKind::Letterspaced | FontKind::Expanded
    ) {
        let what = if eng.stomach.fonts.kind(source) == FontKind::Letterspaced {
            "cannot copy a letterspaced font"
        } else {
            "cannot copy an expanded font"
        };
        super::pdftex::pdf_error(eng, "\\pdfcopyfont", what)?;
        return Ok(None);
    }
    if eng.stomach.font_codes.expand.contains_key(&source) {
        super::pdftex::pdf_error(eng, "\\pdfcopyfont", "cannot copy an expanded font")?;
        return Ok(None);
    }
    let Some(base_tfm) = eng.stomach.fonts.get(source).cloned() else {
        return Ok(None);
    };
    let base_info = eng.fonts().get(source).cloned();
    let Some(base_info) = base_info else {
        return Ok(None);
    };
    let base_name = eng.stomach.fonts.name(source).to_owned();
    let f = eng.fonts().len();
    let info = FontInfo {
        name: base_info.name.clone(),
        file: base_info.file.clone(),
        size: base_info.size,
        dsize: base_info.dsize,
        hyphen_char: base_info.hyphen_char,
        skew_char: base_info.skew_char,
        params: base_info.params.clone(),
        ..FontInfo::null()
    };
    eng.stomach.fonts.insert_kind(f, base_tfm, base_name, FontKind::Copied);
    Ok(Some(info))
}

/// The `\fontdimen` parameters of `f` as the mouth holds them (assignments included).
pub(crate) fn param(eng: &Eng, f: usize, n: usize) -> Scaled {
    eng.fonts().get(f).and_then(|i| i.params.get(n)).copied().unwrap_or(0)
}

/// `char_warning` (§581).
pub(crate) fn char_warning(eng: &mut Eng, f: usize, c: u32) {
    let tracing = eng.int_par(TRACING_LOST_CHARS);
    if tracing <= 0 {
        return;
    }
    let old = eng.int_par(TRACING_ONLINE);
    let online = rtex_core::Loc::IntPar(TRACING_ONLINE as u16);
    if tracing > 1 {
        eng.assign_in_place(online, rtex_core::Value::Int(1));
    }
    eng.begin_diagnostic();
    eng.print_nl("Missing character: There is no ");
    eng.print(c);
    eng.print_str(" in font ");
    let name: Vec<u32> = eng.fonts().get(f).map(|i| i.name.clone()).unwrap_or_default();
    eng.slow_print(&name);
    eng.print_char(u32::from('!'));
    eng.end_diagnostic(false);
    if tracing > 1 {
        eng.assign_in_place(online, rtex_core::Value::Int(old));
    }
}

/// `new_character` (§582): a character node, or a warning and nothing.
pub(crate) fn new_character(eng: &mut Eng, f: usize, c: u32) -> Option<Node> {
    if eng.stomach.fonts.has_char(f, c) {
        Some(Node::Char { font: f, ch: c })
    } else {
        char_warning(eng, f, c);
        None
    }
}
