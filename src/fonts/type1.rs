//! Type 1 font programs (PFB or PFA): parsing, eexec decryption and subsetting for embedding.

use std::collections::BTreeSet;

const EEXEC_KEY: u16 = 55665;
const CHARSTRING_KEY: u16 = 4330;
const C1: u16 = 52845;
const C2: u16 = 22719;

/// A Type 1 font split as PDF's `FontFile` wants it.
#[derive(Clone, Debug, PartialEq)]
pub struct Type1 {
    /// The cleartext part, up to and including `currentfile eexec`.
    pub cleartext: Vec<u8>,
    /// The decrypted private part.
    pub private: Vec<u8>,
    /// `/FontName`.
    pub font_name: String,
    /// `/FontBBox`.
    pub bbox: [f64; 4],
    /// `/ItalicAngle`.
    pub italic_angle: f64,
    /// The built-in encoding: glyph name per code.
    pub encoding: Vec<Option<String>>,
}

/// Why a font program could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadType1;

fn decrypt(data: &[u8], key: u16) -> Vec<u8> {
    let mut r = key;
    data.iter()
        .map(|&c| {
            let p = c ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(C1).wrapping_add(C2);
            p
        })
        .collect()
}

fn encrypt(data: &[u8], key: u16) -> Vec<u8> {
    let mut r = key;
    data.iter()
        .map(|&p| {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(C1).wrapping_add(C2);
            c
        })
        .collect()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn split_pfb(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), BadType1> {
    let mut ascii = Vec::new();
    let mut binary = Vec::new();
    let mut i = 0;
    let mut seen_binary = false;
    while i + 1 < data.len() && data[i] == 0x80 {
        let kind = data[i + 1];
        if kind == 3 {
            break;
        }
        let len = data.get(i + 2..i + 6).ok_or(BadType1)?;
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let seg = data.get(i + 6..i + 6 + len).ok_or(BadType1)?;
        match kind {
            1 if !seen_binary => ascii.extend_from_slice(seg),
            2 => {
                seen_binary = true;
                binary.extend_from_slice(seg);
            }
            _ => {}
        }
        i += 6 + len;
    }
    Ok((ascii, binary))
}

fn split_pfa(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), BadType1> {
    let at = find(data, b"eexec").ok_or(BadType1)? + 5;
    let mut start = at;
    while start < data.len() && matches!(data[start], b'\r' | b'\n' | b' ' | b'\t') {
        start += 1;
    }
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let digits: Vec<u8> = data[start..].iter().copied().filter_map(hex).collect();
    let mut binary: Vec<u8> = digits
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| c[0] * 16 + c[1])
        .collect();
    if let Some(end) = find(&decrypt(&binary, EEXEC_KEY), b"closefile") {
        binary.truncate(end + 9);
    }
    let mut ascii = data[..at].to_vec();
    ascii.push(b'\n');
    Ok((ascii, binary))
}

fn token_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    Some(text[at..].trim_start())
}

fn number_list(s: &str) -> Vec<f64> {
    let open = s.find(['[', '{']).unwrap_or(0);
    let close = s[open..].find([']', '}']).map_or(s.len(), |c| c + open);
    s[open + 1..close]
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect()
}

fn parse_encoding(text: &str) -> Vec<Option<String>> {
    let mut enc = vec![None; 256];
    let Some(at) = text.find("/Encoding") else { return enc };
    let rest = text[at + 9..].trim_start();
    if rest.starts_with("StandardEncoding") {
        for (c, name) in STANDARD_ENCODING.iter().enumerate() {
            if !name.is_empty() {
                enc[c] = Some((*name).to_owned());
            }
        }
        return enc;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    for w in words.windows(4) {
        if w[0] == "dup"
            && w[3] == "put"
            && let (Ok(code), Some(name)) = (w[1].parse::<usize>(), w[2].strip_prefix('/'))
            && code < 256
        {
            enc[code] = Some(name.to_owned());
        }
        if w[0] == "readonly" || w[0] == "def" && w.get(1) == Some(&"currentdict") {
            break;
        }
    }
    enc
}

impl Type1 {
    /// Reads a PFB or PFA font program.
    pub fn parse(data: &[u8]) -> Result<Type1, BadType1> {
        let (cleartext, binary) = if data.first() == Some(&0x80) {
            split_pfb(data)?
        } else {
            split_pfa(data)?
        };
        if binary.len() < 4 {
            return Err(BadType1);
        }
        let private = decrypt(&binary, EEXEC_KEY);
        let text = String::from_utf8_lossy(&cleartext);
        let font_name = token_after(&text, "/FontName")
            .and_then(|s| s.strip_prefix('/'))
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("Unknown")
            .to_owned();
        let bbox = token_after(&text, "/FontBBox").map(number_list).unwrap_or_default();
        let bbox = if bbox.len() == 4 {
            [bbox[0], bbox[1], bbox[2], bbox[3]]
        } else {
            [0.0; 4]
        };
        let italic_angle = token_after(&text, "/ItalicAngle")
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let encoding = parse_encoding(&text);
        Ok(Type1 {
            cleartext,
            private,
            font_name,
            bbox,
            italic_angle,
            encoding,
        })
    }

    /// `/StdVW` from the private dictionary, the dominant vertical stem width.
    pub fn std_vw(&self) -> Option<f64> {
        let text = String::from_utf8_lossy(&self.private);
        token_after(&text, "/StdVW").map(number_list)?.first().copied()
    }

    fn len_iv(&self) -> usize {
        let text = String::from_utf8_lossy(&self.private);
        token_after(&text, "/lenIV")
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(4)
    }

    /// The glyph names in `/CharStrings`, in order.
    pub fn glyph_names(&self) -> Vec<String> {
        self.charstrings()
            .map(|(_, entries, _)| entries.into_iter().map(|e| e.name).collect())
            .unwrap_or_default()
    }

    fn charstrings(&self) -> Option<(usize, Vec<CharString>, usize)> {
        let p = &self.private;
        let start = find(p, b"/CharStrings")?;
        let mut i = start + find(&p[start..], b"begin")? + 5;
        let mut entries = Vec::new();
        loop {
            while i < p.len() && p[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= p.len() {
                return None;
            }
            if p[i] != b'/' {
                return Some((start, entries, i));
            }
            let from = i;
            let word = |i: &mut usize| {
                while *i < p.len() && p[*i].is_ascii_whitespace() {
                    *i += 1;
                }
                let s = *i;
                while *i < p.len() && !p[*i].is_ascii_whitespace() {
                    *i += 1;
                }
                String::from_utf8_lossy(&p[s..*i]).into_owned()
            };
            let name = word(&mut i)[1..].to_owned();
            let len: usize = word(&mut i).parse().ok()?;
            let _rd = word(&mut i);
            i += 1;
            let data = p.get(i..i + len)?.to_vec();
            i += len;
            let line_end = i + p[i..]
                .iter()
                .position(|&c| c == b'\n' || c == b'\r')
                .unwrap_or(p.len() - i);
            i = line_end;
            entries.push(CharString {
                name,
                data,
                text: p[from..line_end].to_vec(),
            });
        }
    }

    /// The `seac` components (base and accent glyph names) of glyph `name`.
    fn seac_components(&self, data: &[u8], len_iv: usize) -> Vec<String> {
        let plain = decrypt(data, CHARSTRING_KEY);
        let ops = plain.get(len_iv..).unwrap_or_default();
        let mut stack: Vec<i32> = Vec::new();
        let mut i = 0;
        while i < ops.len() {
            let v = ops[i];
            match v {
                32..=246 => {
                    stack.push(i32::from(v) - 139);
                    i += 1;
                }
                247..=250 => {
                    let w = i32::from(*ops.get(i + 1).unwrap_or(&0));
                    stack.push((i32::from(v) - 247) * 256 + w + 108);
                    i += 2;
                }
                251..=254 => {
                    let w = i32::from(*ops.get(i + 1).unwrap_or(&0));
                    stack.push(-(i32::from(v) - 251) * 256 - w - 108);
                    i += 2;
                }
                255 => {
                    let b = ops.get(i + 1..i + 5).unwrap_or(&[0, 0, 0, 0]);
                    stack.push(i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
                    i += 5;
                }
                12 => {
                    if ops.get(i + 1) == Some(&6) && stack.len() >= 2 {
                        let a = stack[stack.len() - 1];
                        let b = stack[stack.len() - 2];
                        return [b, a]
                            .iter()
                            .filter_map(|&c| STANDARD_ENCODING.get(c as usize))
                            .filter(|n| !n.is_empty())
                            .map(|n| (*n).to_owned())
                            .collect();
                    }
                    stack.clear();
                    i += 2;
                }
                _ => {
                    stack.clear();
                    i += 1;
                }
            }
        }
        Vec::new()
    }

    /// A copy keeping only `used` glyphs (plus `.notdef` and `seac` components), named `name`.
    pub fn subset(&self, used: &BTreeSet<String>, name: &str) -> Type1 {
        let Some((start, entries, end)) = self.charstrings() else {
            return self.clone();
        };
        let len_iv = self.len_iv();
        let mut keep: BTreeSet<String> = used.clone();
        keep.insert(".notdef".to_owned());
        for e in &entries {
            if used.contains(&e.name) {
                keep.extend(self.seac_components(&e.data, len_iv));
            }
        }
        let kept: Vec<&CharString> = entries.iter().filter(|e| keep.contains(&e.name)).collect();
        let p = &self.private;
        let mut private = p[..start].to_vec();
        private.extend_from_slice(format!("/CharStrings {} dict dup begin\n", kept.len()).as_bytes());
        for e in kept {
            private.extend_from_slice(&e.text);
            private.push(b'\n');
        }
        private.extend_from_slice(&p[end..]);
        let old = format!("/FontName /{}", self.font_name);
        let new = format!("/FontName /{name}");
        let cleartext = match find(&self.cleartext, old.as_bytes()) {
            Some(at) => [&self.cleartext[..at], new.as_bytes(), &self.cleartext[at + old.len()..]].concat(),
            None => self.cleartext.clone(),
        };
        Type1 {
            cleartext,
            private,
            font_name: name.to_owned(),
            ..self.clone()
        }
    }

    /// The `FontFile` stream data with its `Length1` and `Length2` (no trailer, `Length3` 0).
    pub fn font_file(&self) -> (Vec<u8>, usize, usize) {
        let binary = encrypt(&self.private, EEXEC_KEY);
        let mut data = self.cleartext.clone();
        let len1 = data.len();
        data.extend_from_slice(&binary);
        (data, len1, binary.len())
    }
}

#[derive(Clone, Debug)]
struct CharString {
    name: String,
    data: Vec<u8>,
    text: Vec<u8>,
}

/// Adobe StandardEncoding, by code.
pub const STANDARD_ENCODING: [&str; 256] = {
    let mut t = [""; 256];
    let names: [(usize, &str); 149] = [
        (32, "space"),
        (33, "exclam"),
        (34, "quotedbl"),
        (35, "numbersign"),
        (36, "dollar"),
        (37, "percent"),
        (38, "ampersand"),
        (39, "quoteright"),
        (40, "parenleft"),
        (41, "parenright"),
        (42, "asterisk"),
        (43, "plus"),
        (44, "comma"),
        (45, "hyphen"),
        (46, "period"),
        (47, "slash"),
        (48, "zero"),
        (49, "one"),
        (50, "two"),
        (51, "three"),
        (52, "four"),
        (53, "five"),
        (54, "six"),
        (55, "seven"),
        (56, "eight"),
        (57, "nine"),
        (58, "colon"),
        (59, "semicolon"),
        (60, "less"),
        (61, "equal"),
        (62, "greater"),
        (63, "question"),
        (64, "at"),
        (65, "A"),
        (66, "B"),
        (67, "C"),
        (68, "D"),
        (69, "E"),
        (70, "F"),
        (71, "G"),
        (72, "H"),
        (73, "I"),
        (74, "J"),
        (75, "K"),
        (76, "L"),
        (77, "M"),
        (78, "N"),
        (79, "O"),
        (80, "P"),
        (81, "Q"),
        (82, "R"),
        (83, "S"),
        (84, "T"),
        (85, "U"),
        (86, "V"),
        (87, "W"),
        (88, "X"),
        (89, "Y"),
        (90, "Z"),
        (91, "bracketleft"),
        (92, "backslash"),
        (93, "bracketright"),
        (94, "asciicircum"),
        (95, "underscore"),
        (96, "quoteleft"),
        (97, "a"),
        (98, "b"),
        (99, "c"),
        (100, "d"),
        (101, "e"),
        (102, "f"),
        (103, "g"),
        (104, "h"),
        (105, "i"),
        (106, "j"),
        (107, "k"),
        (108, "l"),
        (109, "m"),
        (110, "n"),
        (111, "o"),
        (112, "p"),
        (113, "q"),
        (114, "r"),
        (115, "s"),
        (116, "t"),
        (117, "u"),
        (118, "v"),
        (119, "w"),
        (120, "x"),
        (121, "y"),
        (122, "z"),
        (123, "braceleft"),
        (124, "bar"),
        (125, "braceright"),
        (126, "asciitilde"),
        (161, "exclamdown"),
        (162, "cent"),
        (163, "sterling"),
        (164, "fraction"),
        (165, "yen"),
        (166, "florin"),
        (167, "section"),
        (168, "currency"),
        (169, "quotesingle"),
        (170, "quotedblleft"),
        (171, "guillemotleft"),
        (172, "guilsinglleft"),
        (173, "guilsinglright"),
        (174, "fi"),
        (175, "fl"),
        (177, "endash"),
        (178, "dagger"),
        (179, "daggerdbl"),
        (180, "periodcentered"),
        (182, "paragraph"),
        (183, "bullet"),
        (184, "quotesinglbase"),
        (185, "quotedblbase"),
        (186, "quotedblright"),
        (187, "guillemotright"),
        (188, "ellipsis"),
        (189, "perthousand"),
        (191, "questiondown"),
        (193, "grave"),
        (194, "acute"),
        (195, "circumflex"),
        (196, "tilde"),
        (197, "macron"),
        (198, "breve"),
        (199, "dotaccent"),
        (200, "dieresis"),
        (202, "ring"),
        (203, "cedilla"),
        (205, "hungarumlaut"),
        (206, "ogonek"),
        (207, "caron"),
        (208, "emdash"),
        (225, "AE"),
        (227, "ordfeminine"),
        (232, "Lslash"),
        (233, "Oslash"),
        (234, "OE"),
        (235, "ordmasculine"),
        (241, "ae"),
        (245, "dotlessi"),
        (248, "lslash"),
        (249, "oslash"),
        (250, "oe"),
        (251, "germandbls"),
    ];
    let mut i = 0;
    while i < names.len() {
        t[names[i].0] = names[i].1;
        i += 1;
    }
    t
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eexec_round_trips() {
        let plain = b"abcd/Private 8 dict dup begin";
        assert_eq!(decrypt(&encrypt(plain, EEXEC_KEY), EEXEC_KEY), plain);
    }

    #[test]
    fn standard_encoding_has_the_usual_names() {
        assert_eq!(STANDARD_ENCODING[65], "A");
        assert_eq!(STANDARD_ENCODING[174], "fi");
        assert_eq!(STANDARD_ENCODING[251], "germandbls");
        assert_eq!(STANDARD_ENCODING[0], "");
    }

    #[test]
    fn reads_and_subsets_computer_modern() {
        let Ok(out) = std::process::Command::new("kpsewhich").arg("cmr10.pfb").output() else {
            return;
        };
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let Ok(data) = std::fs::read(&path) else { return };
        let font = Type1::parse(&data).unwrap();
        assert_eq!(font.font_name, "CMR10");
        assert_eq!(font.encoding[65].as_deref(), Some("A"));
        assert!(font.glyph_names().len() > 100);
        let used: BTreeSet<String> = ["A", "b"].iter().map(|s| s.to_string()).collect();
        let sub = font.subset(&used, "ABCDEF+CMR10");
        let mut names = sub.glyph_names();
        names.sort();
        assert_eq!(names, vec![".notdef", "A", "b"]);
        let (data, l1, l2) = sub.font_file();
        assert_eq!(data.len(), l1 + l2);
        assert_eq!(decrypt(&data[l1..], EEXEC_KEY), sub.private);
        assert!(String::from_utf8_lossy(&data[..l1]).contains("/FontName /ABCDEF+CMR10"));
    }
}
