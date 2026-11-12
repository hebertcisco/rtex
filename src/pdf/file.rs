//! Low-level PDF serialization: numbered objects, streams, the cross-reference table and trailer.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use miniz_oxide::deflate::compress_to_vec_zlib;

/// A PDF file being assembled, objects keyed by number.
#[derive(Default)]
pub(crate) struct PdfFile {
    objects: BTreeMap<usize, Vec<u8>>,
    next: usize,
    compress: u8,
}

impl PdfFile {
    /// A file whose fresh numbers start after `reserved`, compressing streams at `level` (0 to 9).
    pub fn new(reserved: usize, level: i32) -> Self {
        PdfFile {
            objects: BTreeMap::new(),
            next: reserved,
            compress: level.clamp(0, 9) as u8,
        }
    }

    /// A fresh object number.
    pub fn alloc(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    /// Writes object `n` with the given body.
    pub fn object(&mut self, n: usize, body: impl Into<Vec<u8>>) {
        self.objects.insert(n, body.into());
    }

    /// Writes stream object `n`: `dict` holds the entries besides `/Length` and `/Filter`.
    pub fn stream(&mut self, n: usize, dict: &str, data: &[u8]) {
        let (data, filter) = if self.compress > 0 && !data.is_empty() {
            (compress_to_vec_zlib(data, self.compress), "/Filter /FlateDecode ")
        } else {
            (data.to_vec(), "")
        };
        self.raw_stream(n, &format!("{filter}{dict}"), &data);
    }

    /// Writes stream object `n` with data already encoded as `dict` says.
    pub fn raw_stream(&mut self, n: usize, dict: &str, data: &[u8]) {
        let mut body = format!("<<\n/Length {}\n{dict}\n>>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.objects.insert(n, body);
    }

    /// Whether object `n` was written.
    pub fn has(&self, n: usize) -> bool {
        self.objects.contains_key(&n)
    }

    /// The finished file: header, objects, cross-references and the trailer `extra` entries.
    pub fn finish(mut self, minor_version: i32, trailer: &str) -> Vec<u8> {
        let size = self.objects.keys().max().copied().unwrap_or(0).max(self.next) + 1;
        for n in 1..size {
            self.objects.entry(n).or_insert_with(|| b"null".to_vec());
        }
        let mut out = format!("%PDF-1.{}\n", minor_version.clamp(0, 9)).into_bytes();
        out.extend_from_slice(b"%\xd0\xd4\xc5\xd8\n");
        let mut offsets = vec![0usize; size];
        for (n, body) in &self.objects {
            offsets[*n] = out.len();
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        let mut table = format!("xref\n0 {size}\n0000000000 65535 f \n");
        for off in offsets.iter().skip(1) {
            let _ = writeln!(table, "{off:010} 00000 n ");
        }
        out.extend_from_slice(table.as_bytes());
        out.extend_from_slice(
            format!("trailer\n<<\n/Size {size}\n{trailer}\n>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }
}

/// pdfTeX's `pdf_print_real`: `v` with up to `digits` decimals, trailing zeros dropped.
pub(crate) fn real(v: f64, digits: usize) -> String {
    let k = 10f64.powi(digits as i32);
    let r = (v * k).round() / k;
    let mut s = format!("{r:.digits$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_owned() } else { s }
}

/// Scaled points to big points.
pub(crate) fn bp(sp: i32) -> f64 {
    f64::from(sp) / 65781.76
}

/// A PDF string for text pdfTeX passes through `pdf_print_str`: kept when it is already a
/// literal or hex string, else wrapped in parentheses.
pub(crate) fn pdf_str(s: &str) -> String {
    if s.is_empty() {
        return "()".to_owned();
    }
    if s.starts_with('(') && s.ends_with(')') {
        return s.to_owned();
    }
    let hex = s.starts_with('<')
        && s.ends_with('>')
        && s.len().is_multiple_of(2)
        && s[1..s.len() - 1].chars().all(|c| c.is_ascii_hexdigit());
    if hex { s.to_owned() } else { format!("({s})") }
}

/// A PDF literal string with `(`, `)` and `\` escaped and other bytes kept.
pub(crate) fn escaped(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_drop_trailing_zeros() {
        assert_eq!(real(1.5, 3), "1.5");
        assert_eq!(real(2.0, 3), "2");
        assert_eq!(real(-0.0001, 3), "0");
        assert_eq!(real(9.96264, 4), "9.9626");
    }

    #[test]
    fn strings_are_wrapped_once() {
        assert_eq!(pdf_str("Title"), "(Title)");
        assert_eq!(pdf_str("(Title)"), "(Title)");
        assert_eq!(pdf_str("<FEFF0041>"), "<FEFF0041>");
        assert_eq!(escaped("a(b)"), "(a\\(b\\))");
    }

    #[test]
    fn files_have_a_valid_xref() {
        let mut f = PdfFile::new(0, 0);
        let n = f.alloc();
        f.object(n, "<< /Type /Catalog >>");
        let pdf = f.finish(7, "/Root 1 0 R");
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(doc.objects.len(), 1);
    }
}
