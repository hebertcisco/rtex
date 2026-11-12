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

/// The metrics of every loaded font; index 0 is `\nullfont`.
#[derive(Clone, Debug, Default)]
pub struct FontStore {
    fonts: Vec<Option<Tfm>>,
    names: Vec<String>,
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
        if self.fonts.len() <= f {
            self.fonts.resize(f + 1, None);
            self.names.resize(f + 1, String::new());
        }
        self.fonts[f] = Some(tfm);
        self.names[f] = name;
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

/// Creates a letterspaced copy of font `source` (`\letterspacefont`).
pub(crate) fn letterspace_font(eng: &mut Eng, source: usize, e: i32) -> Res<Option<FontInfo>> {
    let Some(base_tfm) = eng.stomach.fonts.get(source).cloned() else {
        return Ok(None);
    };
    let base_info = eng.fonts().get(source).cloned();
    let Some(base_info) = base_info else {
        return Ok(None);
    };
    let tfm = base_tfm.letterspaced(e);
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
    eng.stomach.fonts.insert(f, tfm, base_name);
    Ok(Some(info))
}

/// Creates a copy of font `source` (`\pdfcopyfont`).
pub(crate) fn copy_font(eng: &mut Eng, source: usize) -> Res<Option<FontInfo>> {
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
    eng.stomach.fonts.insert(f, base_tfm, base_name);
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
