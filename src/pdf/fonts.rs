//! Font resources: Type 1 fonts from `pdftex.map`, subset and embedded, with widths from the
//! TFM, optional encodings and pdfTeX's ToUnicode CMaps.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use super::file::{PdfFile, real};
use crate::document::{DocFont, Document, DrawOp, FontId};
use crate::fonts::library::FontLibrary;
use crate::fonts::map::Inclusion;
use crate::fonts::type1::Type1;
use crate::units::Scaled;

/// What pdfTeX's font output needs from the job.
pub(crate) struct FontOptions<'a> {
    /// `\pdfgentounicode`.
    pub gen_tounicode: bool,
    /// `\pdfglyphtounicode` definitions.
    pub to_unicode: &'a HashMap<String, String>,
    /// `\pdffontattr` per font.
    pub attrs: &'a HashMap<usize, String>,
    /// Object numbers fixed by `\pdffontobjnum`.
    pub objects: &'a HashMap<usize, usize>,
    /// `\pdfomitcharset`.
    pub omit_charset: bool,
}

struct Embedded {
    first_user: FontId,
    glyphs: BTreeSet<String>,
    inclusion: Inclusion,
    flags: u32,
    name: String,
    descriptor: usize,
    file: usize,
}

/// A deterministic six-letter subset tag, from the font name and the glyphs kept.
fn subset_tag(name: &str, glyphs: &BTreeSet<String>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes().chain(glyphs.iter().flat_map(|g| g.bytes().chain(*b"/"))) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((h >> (i * 10)) % 26) as u8))
        .collect()
}

fn per_mille(v: Scaled, size: Scaled) -> f64 {
    f64::from(v) * 1000.0 / f64::from(size.max(1))
}

/// The Unicode text of glyph `name`: `\pdfglyphtounicode`, then `uniXXXX`, `uXXXX` and `a_b` names.
fn glyph_unicode(name: &str, table: &HashMap<String, String>) -> Option<String> {
    if let Some(u) = table.get(name) {
        return Some(u.split_whitespace().collect::<Vec<_>>().join(""));
    }
    let base = name.split('.').next().unwrap_or(name);
    if let Some(u) = table.get(base) {
        return Some(u.split_whitespace().collect());
    }
    if base.contains('_') {
        let parts: Option<Vec<String>> = base.split('_').map(|p| glyph_unicode(p, table)).collect();
        return parts.map(|p| p.concat());
    }
    if let Some(hex) = base.strip_prefix("uni")
        && hex.len() >= 4
        && hex.len() % 4 == 0
        && hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some(hex.to_ascii_uppercase());
    }
    if let Some(hex) = base.strip_prefix('u')
        && (4..=6).contains(&hex.len())
        && hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        let v = u32::from_str_radix(hex, 16).ok()?;
        let c = char::from_u32(v)?;
        let mut buf = [0u16; 2];
        return Some(c.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect());
    }
    None
}

fn to_unicode_cmap(tfm: &str, entries: &BTreeMap<u32, String>) -> String {
    let mut s = String::new();
    let name = format!("TeX-{tfm}-0");
    let _ = write!(
        s,
        "%!PS-Adobe-3.0 Resource-CMap\n%%DocumentNeededResources: ProcSet (CIDInit)\n%%IncludeResource: ProcSet (CIDInit)\n%%BeginResource: CMap ({name})\n%%Title: ({name} TeX {tfm} 0)\n%%Version: 1.000\n%%EndComments\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<< /Registry (TeX)\n/Ordering ({tfm})\n/Supplement 0\n>> def\n/CMapName /{name} def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
    );
    let list: Vec<(&u32, &String)> = entries.iter().collect();
    for chunk in list.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (c, u) in chunk {
            let _ = writeln!(s, "<{:02X}> <{u}>", **c);
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n%%EndResource\n%%EOF\n");
    s
}

/// Writes every font the pages and forms use; returns the object number of each font resource.
pub(crate) fn write_fonts<'a>(
    file: &mut PdfFile,
    document: &Document,
    op_lists: impl Iterator<Item = &'a [DrawOp]>,
    library: &FontLibrary,
    options: &FontOptions,
) -> BTreeMap<FontId, usize> {
    let mut widths: BTreeMap<FontId, BTreeMap<u32, Scaled>> = BTreeMap::new();
    for op in op_lists.flatten() {
        if let DrawOp::Text {
            font, codes, advances, ..
        } = op
        {
            let w = widths.entry(*font).or_default();
            for (c, a) in codes.iter().zip(advances) {
                w.insert(*c, *a);
            }
        }
    }
    let mut ids = BTreeMap::new();
    let mut embedded: BTreeMap<String, Embedded> = BTreeMap::new();
    for (&font, w) in &widths {
        let Some(doc) = document.font(font) else { continue };
        let id = options.objects.get(&font).copied().unwrap_or_else(|| file.alloc());
        ids.insert(font, id);
        let entry = library.entry(&doc.name);
        if let Some((f, inclusion)) = entry
            .and_then(|e| e.font_file.clone())
            .filter(|(f, _)| library.program(f).is_some())
        {
            let e = embedded.entry(f).or_insert_with(|| Embedded {
                first_user: font,
                glyphs: BTreeSet::new(),
                inclusion,
                flags: entry.and_then(|e| e.flags).unwrap_or(4),
                name: String::new(),
                descriptor: file.alloc(),
                file: file.alloc(),
            });
            for &c in w.keys() {
                if let Some(g) = library.glyph_name(&doc.name, c) {
                    e.glyphs.insert(g);
                }
            }
        }
    }
    for (f, e) in &mut embedded {
        let Some(program) = library.program(f) else { continue };
        e.name = match e.inclusion {
            Inclusion::Subset => format!("{}+{}", subset_tag(&program.font_name, &e.glyphs), program.font_name),
            Inclusion::Full => program.font_name.clone(),
        };
    }
    for (&font, w) in &widths {
        let Some(doc) = document.font(font) else { continue };
        let entry = library.entry(&doc.name);
        let emb = entry
            .and_then(|e| e.font_file.as_ref())
            .and_then(|(f, _)| embedded.get(f));
        write_font(file, ids[&font], font, doc, w, emb, library, options);
    }
    for (f, e) in &embedded {
        let (Some(program), Some(doc)) = (library.program(f), document.font(e.first_user)) else {
            continue;
        };
        write_embedded(file, doc, e, program, options);
    }
    ids
}

#[allow(clippy::too_many_arguments)]
fn write_font(
    file: &mut PdfFile,
    id: usize,
    font: FontId,
    doc: &DocFont,
    widths: &BTreeMap<u32, Scaled>,
    embedded: Option<&Embedded>,
    library: &FontLibrary,
    options: &FontOptions,
) {
    let first = widths.keys().next().copied().unwrap_or(0);
    let last = widths.keys().last().copied().unwrap_or(0).min(255);
    let base = match (embedded, library.entry(&doc.name)) {
        (Some(e), _) => e.name.clone(),
        (None, Some(entry)) => entry.ps_name.clone().unwrap_or_else(|| doc.name.to_ascii_uppercase()),
        (None, None) => doc.name.to_ascii_uppercase(),
    };
    let w: Vec<String> = (first..=last)
        .map(|c| {
            widths
                .get(&c)
                .map_or("0".to_owned(), |w| real(per_mille(*w, doc.size), 1))
        })
        .collect();
    let mut d = format!(
        "<<\n/Type /Font\n/Subtype /Type1\n/BaseFont /{base}\n/FirstChar {first}\n/LastChar {last}\n/Widths [{}]\n",
        w.join(" ")
    );
    if let Some(e) = embedded {
        let _ = writeln!(d, "/FontDescriptor {} 0 R", e.descriptor);
    }
    if let Some(enc) = library.encoding(&doc.name) {
        d.push_str("/Encoding << /Type /Encoding /Differences [");
        let mut prev: Option<u32> = None;
        for &c in widths.keys().filter(|&&c| c < 256) {
            if prev != Some(c.wrapping_sub(1)) {
                let _ = write!(d, " {c}");
            }
            let _ = write!(d, " /{}", enc[c as usize]);
            prev = Some(c);
        }
        d.push_str(" ] >>\n");
    }
    if options.gen_tounicode {
        let entries: BTreeMap<u32, String> = widths
            .keys()
            .filter_map(|&c| {
                let g = library.glyph_name(&doc.name, c)?;
                Some((c, glyph_unicode(&g, options.to_unicode)?))
            })
            .collect();
        if !entries.is_empty() {
            let cmap = file.alloc();
            file.stream(cmap, "", to_unicode_cmap(&doc.name, &entries).as_bytes());
            let _ = writeln!(d, "/ToUnicode {cmap} 0 R");
        }
    }
    if let Some(a) = options.attrs.get(&font) {
        d.push_str(a);
        d.push('\n');
    }
    d.push_str(">>");
    file.object(id, d);
}

fn write_embedded(file: &mut PdfFile, doc: &DocFont, e: &Embedded, program: &Type1, options: &FontOptions) {
    let font = match e.inclusion {
        Inclusion::Subset => program.subset(&e.glyphs, &e.name),
        Inclusion::Full => program.clone(),
    };
    let (data, len1, len2) = font.font_file();
    let m = doc.metrics;
    let [x0, y0, x1, y1] = program.bbox;
    let mut d = format!(
        "<<\n/Type /FontDescriptor\n/FontName /{}\n/Flags {}\n/FontBBox [{} {} {} {}]\n/Ascent {}\n/CapHeight {}\n/Descent {}\n/ItalicAngle {}\n/StemV {}\n/XHeight {}\n",
        e.name,
        e.flags,
        real(x0, 0),
        real(y0, 0),
        real(x1, 0),
        real(y1, 0),
        real(per_mille(m.ascent, doc.size), 0),
        real(per_mille(m.cap_height, doc.size), 0),
        real(-per_mille(m.descent, doc.size), 0),
        real(program.italic_angle, 4),
        real(program.std_vw().unwrap_or(0.0), 0),
        real(per_mille(m.x_height, doc.size), 0),
    );
    if e.inclusion == Inclusion::Subset && !options.omit_charset {
        let set: String = e.glyphs.iter().map(|g| format!("/{g}")).collect();
        let _ = writeln!(d, "/CharSet ({set})");
    }
    let _ = writeln!(d, "/FontFile {} 0 R\n>>", e.file);
    file.object(e.descriptor, d);
    file.stream(e.file, &format!("/Length1 {len1}\n/Length2 {len2}\n/Length3 0"), &data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_names_map_to_unicode() {
        let mut t = HashMap::new();
        t.insert("ff".to_owned(), "0066 0066".to_owned());
        assert_eq!(glyph_unicode("ff", &t).as_deref(), Some("00660066"));
        assert_eq!(glyph_unicode("uni00E9", &t).as_deref(), Some("00E9"));
        assert_eq!(glyph_unicode("u1F600", &t).as_deref(), Some("D83DDE00"));
        assert_eq!(glyph_unicode("ff_ff", &t).as_deref(), Some("0066006600660066"));
        assert_eq!(glyph_unicode("foo", &t), None);
    }
}
