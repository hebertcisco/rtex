//! Font map files (`pdftex.map`): which PostScript font, encoding and font file stand for a TFM.

use std::collections::HashMap;

/// How a font file is included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inclusion {
    /// Subset to the glyphs used (`<file`).
    Subset,
    /// Embedded whole (`<<file`).
    Full,
}

/// One line of a map file.
#[derive(Clone, Debug, PartialEq)]
pub struct MapEntry {
    /// The TFM name.
    pub tfm: String,
    /// The PostScript name of the font.
    pub ps_name: Option<String>,
    /// The font descriptor flags, if given.
    pub flags: Option<u32>,
    /// `SlantFont` amount.
    pub slant: f64,
    /// `ExtendFont` amount.
    pub extend: f64,
    /// The encoding file (`<file.enc`).
    pub encoding: Option<String>,
    /// The font file and how to include it.
    pub font_file: Option<(String, Inclusion)>,
}

/// The entries of one or more map files, by TFM name; the first entry for a name wins.
#[derive(Clone, Debug, Default)]
pub struct FontMap {
    entries: HashMap<String, MapEntry>,
}

impl FontMap {
    /// Parses a map file and adds its entries.
    pub fn add(&mut self, text: &str) {
        for line in text.lines() {
            if let Some(e) = parse_line(line) {
                self.entries.entry(e.tfm.clone()).or_insert(e);
            }
        }
    }

    /// Applies map lines as `\pdfmapfile`/`\pdfmapline` do: `+` adds new names, `=` replaces,
    /// `-` removes; returns the names `+` found already present.
    pub fn apply(&mut self, op: char, text: &str) -> Vec<String> {
        let mut duplicates = Vec::new();
        for line in text.lines() {
            let Some(e) = parse_line(line) else { continue };
            match op {
                '+' => {
                    if self.entries.contains_key(&e.tfm) {
                        duplicates.push(e.tfm.clone());
                    } else {
                        self.entries.insert(e.tfm.clone(), e);
                    }
                }
                '-' => {
                    self.entries.remove(&e.tfm);
                }
                _ => {
                    self.entries.insert(e.tfm.clone(), e);
                }
            }
        }
        duplicates
    }

    /// The entry for `tfm`.
    pub fn get(&self, tfm: &str) -> Option<&MapEntry> {
        self.entries.get(tfm)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let s: String = chars.by_ref().take_while(|&c| c != '"').collect();
            out.push(format!("\"{s}"));
        } else {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                s.push(c);
                chars.next();
            }
            out.push(s);
        }
    }
    out
}

/// Parses one map line, or `None` for comments and blank lines.
pub fn parse_line(line: &str) -> Option<MapEntry> {
    let line = line.trim();
    if line.is_empty() || matches!(line.as_bytes()[0], b'%' | b'#' | b';' | b'*') {
        return None;
    }
    let toks = tokens(line);
    let mut it = toks.into_iter();
    let tfm = it.next()?;
    let mut e = MapEntry {
        tfm,
        ps_name: None,
        flags: None,
        slant: 0.0,
        extend: 1.0,
        encoding: None,
        font_file: None,
    };
    let mut pending_lt = false;
    for t in it {
        if let Some(special) = t.strip_prefix('"') {
            let words: Vec<&str> = special.split_whitespace().collect();
            for w in words.windows(2) {
                match w[1] {
                    "SlantFont" => e.slant = w[0].parse().unwrap_or(0.0),
                    "ExtendFont" => e.extend = w[0].parse().unwrap_or(1.0),
                    _ => {}
                }
            }
            continue;
        }
        let (rest, lt) = if pending_lt {
            pending_lt = false;
            (t.as_str(), Some(Inclusion::Subset))
        } else if let Some(r) = t.strip_prefix("<<") {
            (r, Some(Inclusion::Full))
        } else if let Some(r) = t.strip_prefix("<[") {
            (r, Some(Inclusion::Subset))
        } else if let Some(r) = t.strip_prefix('<') {
            (r, Some(Inclusion::Subset))
        } else {
            (t.as_str(), None)
        };
        match lt {
            Some(_) if rest.is_empty() => pending_lt = true,
            Some(inc) => {
                if rest.to_ascii_lowercase().ends_with(".enc") {
                    e.encoding = Some(rest.to_owned());
                } else {
                    e.font_file = Some((rest.to_owned(), inc));
                }
            }
            None => {
                if let Ok(n) = rest.parse::<u32>()
                    && e.ps_name.is_some()
                {
                    e.flags = Some(n);
                } else if e.ps_name.is_none() {
                    e.ps_name = Some(rest.to_owned());
                }
            }
        }
    }
    Some(e)
}

/// Parses a PostScript encoding vector (`/Name [ /a /b ... ] def`) into 256 glyph names.
pub fn parse_encoding(text: &str) -> Option<Vec<String>> {
    let body = text
        .lines()
        .map(|l| l.split('%').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let start = body.find('[')?;
    let end = body[start..].find(']')? + start;
    let names: Vec<String> = body[start + 1..end]
        .split_whitespace()
        .filter_map(|n| n.strip_prefix('/'))
        .map(str::to_owned)
        .collect();
    let mut v = names;
    v.resize(256, ".notdef".to_owned());
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pdftex_map_lines() {
        let e = parse_line("cmr10 CMR10 <cmr10.pfb").unwrap();
        assert_eq!(e.ps_name.as_deref(), Some("CMR10"));
        assert_eq!(e.font_file, Some(("cmr10.pfb".into(), Inclusion::Subset)));
        let e = parse_line(
            r#"ptmro8r NimbusRomNo9L-Regu " .167 SlantFont TeXBase1Encoding ReEncodeFont " <8r.enc <utmr8a.pfb"#,
        )
        .unwrap();
        assert_eq!(e.encoding.as_deref(), Some("8r.enc"));
        assert!((e.slant - 0.167).abs() < 1e-9);
        assert_eq!(e.font_file.unwrap().0, "utmr8a.pfb");
        let e = parse_line("ptmr8r Times-Roman 4 <<x.pfa").unwrap();
        assert_eq!(e.flags, Some(4));
        assert_eq!(e.font_file, Some(("x.pfa".into(), Inclusion::Full)));
        assert!(parse_line("% comment").is_none());
    }

    #[test]
    fn parses_encoding_vectors() {
        let v = parse_encoding("% c\n/Enc [ /a /b % x\n /c ] def").unwrap();
        assert_eq!(&v[..3], ["a", "b", "c"]);
        assert_eq!(v.len(), 256);
        assert_eq!(v[3], ".notdef");
    }
}
