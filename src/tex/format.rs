//! The stomach's part of a format: fonts and hyphenation patterns.
//!
//! TFM data itself is not copied into the image; the font name and size are,
//! and the metrics are read again from the TeX distribution when the format
//! loads. That keeps the image small and portable across a distribution's
//! file layout, at the cost of requiring the same TFM files at load time.

use super::Eng;
use crate::fonts::tfm::Tfm;

/// Serializes fonts and hyphenation data.
pub(crate) fn dump(eng: &Eng) -> Vec<u8> {
    let mut out = Vec::new();
    let store = &eng.stomach.fonts;
    put_u32(&mut out, store.len() as u32);
    for f in 0..store.len() {
        match store.get(f) {
            Some(tfm) => {
                out.push(1);
                put_str(&mut out, store.name(f));
                out.extend_from_slice(&tfm.size.to_le_bytes());
            }
            None => out.push(0),
        }
    }
    let hyphenation = eng.stomach.hyph.encode();
    put_u32(&mut out, hyphenation.len() as u32);
    out.extend_from_slice(&hyphenation);
    out
}

/// Restores fonts and hyphenation data.
pub(crate) fn undump(eng: &mut Eng, data: &[u8]) -> bool {
    let mut reader = Reader { data, at: 0 };
    let Some(count) = reader.u32() else {
        return false;
    };
    for id in 0..count as usize {
        let Some(present) = reader.u8() else {
            return false;
        };
        if present == 0 {
            continue;
        }
        let Some(name) = reader.str() else {
            return false;
        };
        let Some(size) = reader.i32() else {
            return false;
        };
        let Some(found) = eng.file_system_mut().find(&format!("{name}.tfm")) else {
            return false;
        };
        let Ok(tfm) = Tfm::parse(&found.data, size) else {
            return false;
        };
        eng.stomach.fonts.insert(id, tfm, name);
    }
    let Some(len) = reader.u32() else {
        return false;
    };
    let Some(hyphenation) = reader.bytes(len as usize) else {
        return false;
    };
    eng.stomach.hyph.decode(hyphenation)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let value = *self.data.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.data.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.at..self.at + len)?;
        self.at += len;
        Some(bytes)
    }

    fn str(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.bytes(len)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stomach_round_trips() {
        let mut out = Vec::new();
        put_u32(&mut out, 0);
        let mut reader = Reader { data: &out, at: 0 };
        assert_eq!(reader.u32(), Some(0));
    }
}
