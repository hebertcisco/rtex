//! A small deterministic ZIP writer, enough for EPUB's OCF container.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Central {
    name: String,
    crc: u32,
    method: u16,
    compressed: u32,
    uncompressed: u32,
    offset: u32,
}

/// Builds a ZIP archive. Entries keep their order; `deflate` compresses an
/// entry, so the OCF `mimetype` entry stays stored and first.
pub fn archive(entries: &[(String, Vec<u8>, bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, deflate) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let (method, payload) = if *deflate {
            (8u16, miniz_oxide::deflate::compress_to_vec(data, 6))
        } else {
            (0u16, data.clone())
        };
        put_u32(&mut out, 0x0403_4b50);
        put_u16(&mut out, 20);
        put_u16(&mut out, 0x0800);
        put_u16(&mut out, method);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u32(&mut out, crc);
        put_u32(&mut out, payload.len() as u32);
        put_u32(&mut out, data.len() as u32);
        put_u16(&mut out, name.len() as u16);
        put_u16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&payload);
        central.push(Central {
            name: name.clone(),
            crc,
            method,
            compressed: payload.len() as u32,
            uncompressed: data.len() as u32,
            offset,
        });
    }
    let directory_offset = out.len() as u32;
    for entry in &central {
        put_u32(&mut out, 0x0201_4b50);
        put_u16(&mut out, 20);
        put_u16(&mut out, 20);
        put_u16(&mut out, 0x0800);
        put_u16(&mut out, entry.method);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u32(&mut out, entry.crc);
        put_u32(&mut out, entry.compressed);
        put_u32(&mut out, entry.uncompressed);
        put_u16(&mut out, entry.name.len() as u16);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u16(&mut out, 0);
        put_u32(&mut out, 0);
        put_u32(&mut out, entry.offset);
        out.extend_from_slice(entry.name.as_bytes());
    }
    let directory_size = out.len() as u32 - directory_offset;
    put_u32(&mut out, 0x0605_4b50);
    put_u16(&mut out, 0);
    put_u16(&mut out, 0);
    put_u16(&mut out, central.len() as u16);
    put_u16(&mut out, central.len() as u16);
    put_u32(&mut out, directory_size);
    put_u32(&mut out, directory_offset);
    put_u16(&mut out, 0);
    out
}

/// Reads back the entries of an archive this module wrote, for tests and tools.
pub fn read(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut i = 0;
    while i + 30 <= archive.len() && archive[i..i + 4] == [0x50, 0x4b, 0x03, 0x04] {
        let method = u16::from_le_bytes([archive[i + 8], archive[i + 9]]);
        let compressed =
            u32::from_le_bytes([archive[i + 18], archive[i + 19], archive[i + 20], archive[i + 21]]) as usize;
        let name_len = u16::from_le_bytes([archive[i + 26], archive[i + 27]]) as usize;
        let extra_len = u16::from_le_bytes([archive[i + 28], archive[i + 29]]) as usize;
        let name_start = i + 30;
        let data_start = name_start + name_len + extra_len;
        let data_end = data_start + compressed;
        if data_end > archive.len() {
            break;
        }
        let name = String::from_utf8_lossy(&archive[name_start..name_start + name_len]).into_owned();
        let payload = &archive[data_start..data_end];
        let data = if method == 8 {
            miniz_oxide::inflate::decompress_to_vec(payload).unwrap_or_default()
        } else {
            payload.to_vec()
        };
        entries.push((name, data));
        i = data_end;
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_starts_with_the_first_local_header() {
        let zip = archive(&[("mimetype".to_owned(), b"application/epub+zip".to_vec(), false)]);
        assert_eq!(&zip[..4], b"PK\x03\x04");
        assert!(zip.windows(8).any(|w| w == b"mimetype"));
        assert!(zip.windows(4).any(|w| w == b"PK\x05\x06"));
    }
}
