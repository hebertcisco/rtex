//! Image XObjects for `\pdfximage`: PNG and JPEG data, and PDF pages imported as forms
//! scaled to the unit square, so all are placed alike.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::file::{PdfFile, real};
use crate::images::{Image, ImageKind};

/// Writes image object `n` with `attr` added to its dictionary.
pub(crate) fn write_image(file: &mut PdfFile, n: usize, image: &Image, attr: &str) {
    let written = match image.kind {
        ImageKind::Png => write_png(file, n, image, attr),
        ImageKind::Jpeg => write_jpeg(file, n, image, attr),
        ImageKind::Pdf => write_pdf_page(file, n, image, attr),
    };
    if written.is_none() {
        file.object(n, "null");
    }
}

fn color_space(channels: usize) -> &'static str {
    match channels {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    }
}

fn write_png(file: &mut PdfFile, n: usize, image: &Image, attr: &str) -> Option<()> {
    let data = &image.data;
    let decoder = png::Decoder::new(std::io::Cursor::new(&data[..]));
    let reader = decoder.read_info().ok()?;
    let info = reader.info().clone();
    let simple = !info.interlaced
        && info.trns.is_none()
        && matches!(
            info.color_type,
            png::ColorType::Grayscale | png::ColorType::Rgb | png::ColorType::Indexed
        );
    let bits = info.bit_depth as u8;
    let (w, h) = (info.width, info.height);
    if simple {
        let mut idat = Vec::new();
        let mut i = 8;
        while i + 8 <= data.len() {
            let len = u32::from_be_bytes(data[i..i + 4].try_into().ok()?) as usize;
            if &data[i + 4..i + 8] == b"IDAT" {
                idat.extend_from_slice(data.get(i + 8..i + 8 + len)?);
            }
            i += 12 + len;
        }
        let (cs, colors) = match info.color_type {
            png::ColorType::Indexed => {
                let palette = info.palette.as_ref()?;
                let hex: String = palette.iter().map(|b| format!("{b:02X}")).collect();
                (format!("[/Indexed /DeviceRGB {} <{hex}>]", palette.len() / 3 - 1), 1)
            }
            png::ColorType::Grayscale => ("/DeviceGray".to_owned(), 1),
            _ => ("/DeviceRGB".to_owned(), 3),
        };
        let dict = format!(
            "/Type /XObject\n/Subtype /Image\n/Width {w}\n/Height {h}\n/BitsPerComponent {bits}\n/ColorSpace {cs}\n/Filter /FlateDecode\n/DecodeParms << /Colors {colors} /Columns {w} /BitsPerComponent {bits} /Predictor 10 >>\n{attr}"
        );
        file.raw_stream(n, &dict, &idat);
        return Some(());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&data[..]));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let pixels = &buf[..frame.buffer_size()];
    let (channels, alpha) = match frame.color_type {
        png::ColorType::Grayscale => (1, false),
        png::ColorType::GrayscaleAlpha => (1, true),
        png::ColorType::Rgb => (3, false),
        _ => (3, true),
    };
    let stride = channels + usize::from(alpha);
    let mut color = Vec::with_capacity(pixels.len());
    let mut mask = Vec::new();
    for px in pixels.chunks(stride) {
        color.extend_from_slice(&px[..channels]);
        if alpha {
            mask.push(px[channels]);
        }
    }
    let mut dict = format!(
        "/Type /XObject\n/Subtype /Image\n/Width {w}\n/Height {h}\n/BitsPerComponent 8\n/ColorSpace {}\n",
        color_space(channels)
    );
    if alpha {
        let m = file.alloc();
        file.stream(
            m,
            &format!(
                "/Type /XObject\n/Subtype /Image\n/Width {w}\n/Height {h}\n/BitsPerComponent 8\n/ColorSpace /DeviceGray"
            ),
            &mask,
        );
        let _ = writeln!(dict, "/SMask {m} 0 R");
    }
    dict.push_str(attr);
    file.stream(n, &dict, &color);
    Some(())
}

fn write_jpeg(file: &mut PdfFile, n: usize, image: &Image, attr: &str) -> Option<()> {
    let data = &image.data;
    let mut channels = 3;
    let mut adobe = false;
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if marker == 0xEE && data.get(i + 4..i + 9) == Some(b"Adobe") {
            adobe = true;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            channels = usize::from(*data.get(i + 9)?);
            break;
        }
        i += 2 + len;
    }
    let mut dict = format!(
        "/Type /XObject\n/Subtype /Image\n/Width {}\n/Height {}\n/BitsPerComponent {}\n/ColorSpace {}\n/Filter /DCTDecode\n",
        image.width,
        image.height,
        image.color_depth,
        color_space(channels)
    );
    if channels == 4 && adobe {
        dict.push_str("/Decode [1 0 1 0 1 0 1 0]\n");
    }
    dict.push_str(attr);
    file.raw_stream(n, &dict, data);
    Some(())
}

struct Importer<'a> {
    doc: &'a lopdf::Document,
    map: HashMap<lopdf::ObjectId, usize>,
    queue: Vec<lopdf::ObjectId>,
}

impl Importer<'_> {
    fn reference(&mut self, file: &mut PdfFile, id: lopdf::ObjectId) -> usize {
        if let Some(&n) = self.map.get(&id) {
            return n;
        }
        let n = file.alloc();
        self.map.insert(id, n);
        self.queue.push(id);
        n
    }

    fn write(&mut self, file: &mut PdfFile, o: &lopdf::Object, out: &mut Vec<u8>) {
        use lopdf::Object;
        match o {
            Object::Null => out.extend_from_slice(b"null"),
            Object::Boolean(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Object::Integer(i) => out.extend_from_slice(i.to_string().as_bytes()),
            Object::Real(r) => out.extend_from_slice(real(f64::from(*r), 6).as_bytes()),
            Object::Name(n) => {
                out.push(b'/');
                for &b in n {
                    if b.is_ascii_graphic() && !b"()<>[]{}/%#".contains(&b) {
                        out.push(b);
                    } else {
                        out.extend_from_slice(format!("#{b:02X}").as_bytes());
                    }
                }
            }
            Object::String(s, _) => {
                out.push(b'<');
                for b in s {
                    out.extend_from_slice(format!("{b:02X}").as_bytes());
                }
                out.push(b'>');
            }
            Object::Array(a) => {
                out.push(b'[');
                for (k, x) in a.iter().enumerate() {
                    if k > 0 {
                        out.push(b' ');
                    }
                    self.write(file, x, out);
                }
                out.push(b']');
            }
            Object::Dictionary(d) => self.dict(file, d, out),
            Object::Stream(s) => self.dict(file, &s.dict, out),
            Object::Reference(id) => {
                let n = self.reference(file, *id);
                out.extend_from_slice(format!("{n} 0 R").as_bytes());
            }
        }
    }

    fn dict(&mut self, file: &mut PdfFile, d: &lopdf::Dictionary, out: &mut Vec<u8>) {
        out.extend_from_slice(b"<<");
        for (k, v) in d.iter() {
            if k == b"Length" {
                continue;
            }
            out.push(b' ');
            self.write(file, &lopdf::Object::Name(k.clone()), out);
            out.push(b' ');
            self.write(file, v, out);
        }
        out.extend_from_slice(b" >>");
    }

    fn flush(&mut self, file: &mut PdfFile) {
        while let Some(id) = self.queue.pop() {
            let n = self.map[&id];
            let Ok(o) = self.doc.get_object(id) else {
                file.object(n, "null");
                continue;
            };
            let mut body = Vec::new();
            if let lopdf::Object::Stream(s) = o {
                let mut d = Vec::new();
                for (k, v) in s.dict.iter() {
                    if k == b"Length" {
                        continue;
                    }
                    d.push(b' ');
                    self.write(file, &lopdf::Object::Name(k.clone()), &mut d);
                    d.push(b' ');
                    self.write(file, v, &mut d);
                }
                file.raw_stream(n, &String::from_utf8_lossy(&d), &s.content);
            } else {
                self.write(file, o, &mut body);
                file.object(n, body);
            }
        }
    }
}

fn write_pdf_page(file: &mut PdfFile, n: usize, image: &Image, attr: &str) -> Option<()> {
    let doc = lopdf::Document::load_mem(&image.data).ok()?;
    let pages = doc.get_pages();
    let id = *pages.get(&(image.page as u32))?;
    let content = doc.get_page_content(id);
    let mut imp = Importer {
        doc: &doc,
        map: HashMap::new(),
        queue: Vec::new(),
    };
    let mut resources = Vec::new();
    let mut page = id;
    let res = loop {
        let dict = doc.get_dictionary(page).ok()?;
        if let Ok(r) = dict.get(b"Resources") {
            break Some(r.clone());
        }
        match dict.get(b"Parent").and_then(lopdf::Object::as_reference) {
            Ok(p) => page = p,
            Err(_) => break None,
        }
    };
    match res {
        Some(r) => imp.write(file, &r, &mut resources),
        None => resources.extend_from_slice(b"<< >>"),
    }
    imp.flush(file);
    let [x0, y0, x1, y1] = image.bbox;
    let (w, h) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
    let m = match image.rotate {
        90 => [0.0, -1.0 / w, 1.0 / h, 0.0, -y0 / h, 1.0 + x0 / w],
        180 => [-1.0 / w, 0.0, 0.0, -1.0 / h, 1.0 + x0 / w, 1.0 + y0 / h],
        270 => [0.0, 1.0 / w, -1.0 / h, 0.0, 1.0 + y0 / h, -x0 / w],
        _ => [1.0 / w, 0.0, 0.0, 1.0 / h, -x0 / w, -y0 / h],
    };
    let matrix: Vec<String> = m.iter().map(|v| real(*v, 8)).collect();
    let dict = format!(
        "/Type /XObject\n/Subtype /Form\n/FormType 1\n/BBox [{} {} {} {}]\n/Matrix [{}]\n/Resources {}\n{attr}",
        real(x0, 4),
        real(y0, 4),
        real(x1, 4),
        real(y1, 4),
        matrix.join(" "),
        String::from_utf8_lossy(&resources)
    );
    file.stream(n, &dict, &content);
    Some(())
}
