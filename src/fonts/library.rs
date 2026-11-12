//! The font files a document needs, loaded once: the map, encoding vectors and Type 1 programs.

use std::collections::HashMap;

use rtex_core::FileSystem;

use super::map::{FontMap, MapEntry, parse_encoding};
use super::type1::Type1;

/// Map entries, encodings and font programs for the fonts of a document.
#[derive(Clone, Debug, Default)]
pub struct FontLibrary {
    map: FontMap,
    encodings: HashMap<String, Vec<String>>,
    programs: HashMap<String, Type1>,
}

impl FontLibrary {
    /// A library over a loaded map and the encodings already read.
    pub fn new(map: FontMap, encodings: HashMap<String, Vec<String>>) -> Self {
        FontLibrary {
            map,
            encodings,
            programs: HashMap::new(),
        }
    }

    /// Reads the font programs the map names for `tfms`; returns the names they were found under, sorted.
    pub fn load_programs<'a>(
        &mut self,
        tfms: impl IntoIterator<Item = &'a str>,
        files: &mut dyn FileSystem,
    ) -> Vec<String> {
        let mut shown = std::collections::BTreeMap::new();
        for tfm in tfms {
            let Some((file, _)) = self.map.get(tfm).and_then(|e| e.font_file.clone()) else {
                continue;
            };
            if self.programs.contains_key(&file) {
                continue;
            }
            if let Some(f) = files.find(&file)
                && let Ok(p) = Type1::parse(&f.data)
            {
                self.programs.insert(file.clone(), p);
                shown.insert(file, f.display_name);
            }
        }
        shown.into_values().collect()
    }

    /// Reads `pdftex.map` and the files its entries name, for tools outside a job.
    pub fn load<'a>(tfms: impl IntoIterator<Item = &'a str> + Clone, files: &mut dyn FileSystem) -> Self {
        let mut map = FontMap::default();
        if let Some(f) = files.find("pdftex.map") {
            map.add(&String::from_utf8_lossy(&f.data));
        }
        let mut encodings = HashMap::new();
        for tfm in tfms.clone() {
            if let Some(enc) = map.get(tfm).and_then(|e| e.encoding.clone())
                && let Some(v) = files
                    .find(&enc)
                    .and_then(|f| parse_encoding(&String::from_utf8_lossy(&f.data)))
            {
                encodings.insert(enc, v);
            }
        }
        let mut lib = FontLibrary::new(map, encodings);
        lib.load_programs(tfms, files);
        lib
    }

    /// The map entry of `tfm`.
    pub fn entry(&self, tfm: &str) -> Option<&MapEntry> {
        self.map.get(tfm)
    }

    /// The loaded program of a font file.
    pub fn program(&self, file: &str) -> Option<&Type1> {
        self.programs.get(file)
    }

    /// The glyph name of `code` for `tfm`: from its encoding file, else the program's built-in encoding.
    pub fn glyph_name(&self, tfm: &str, code: u32) -> Option<String> {
        let entry = self.map.get(tfm)?;
        let code = usize::try_from(code).ok().filter(|&c| c < 256)?;
        if let Some(enc) = entry.encoding.as_ref().and_then(|e| self.encodings.get(e)) {
            return Some(enc[code].clone());
        }
        let (file, _) = entry.font_file.as_ref()?;
        self.programs.get(file)?.encoding[code].clone()
    }

    /// The encoding vector of `tfm`, if its map entry names one.
    pub fn encoding(&self, tfm: &str) -> Option<&[String]> {
        let enc = self.map.get(tfm)?.encoding.as_ref()?;
        self.encodings.get(enc).map(Vec::as_slice)
    }
}
