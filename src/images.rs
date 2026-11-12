//! Images for `\pdfximage`: PNG, JPEG and pages of PDF files, with what pdfTeX's
//! `read_image` reports about them.

use std::sync::Arc;

use crate::units::Scaled;

/// The kind of an image file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// Portable Network Graphics.
    Png,
    /// JPEG (JFIF or Exif).
    Jpeg,
    /// A page of a PDF file.
    Pdf,
}

/// Which box of a PDF page to use (`\pdfpagebox`, or `mediabox`, `cropbox`, ... on `\pdfximage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBox {
    /// `mediabox`.
    Media,
    /// `cropbox`.
    Crop,
    /// `bleedbox`.
    Bleed,
    /// `trimbox`.
    Trim,
    /// `artbox`.
    Art,
}

/// An image read for inclusion.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// The kind.
    pub kind: ImageKind,
    /// Width in pixels, or in scaled points for a PDF page.
    pub width: i32,
    /// Height in pixels, or in scaled points for a PDF page.
    pub height: i32,
    /// Horizontal resolution in dots per inch, 0 when unknown.
    pub x_res: i32,
    /// Vertical resolution in dots per inch, 0 when unknown.
    pub y_res: i32,
    /// Bits per colour component.
    pub color_depth: i32,
    /// Number of pages of a PDF, else 1.
    pub pages: i32,
    /// The page included, starting at 1.
    pub page: i32,
    /// The rotation of a PDF page, in degrees.
    pub rotate: i32,
    /// The box of a PDF page in PDF units: lower-left x, y, upper-right x, y.
    pub bbox: [f64; 4],
    /// The file contents.
    pub data: Arc<[u8]>,
}

/// Why an image could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// The format is not one rtex reads.
    #[error("unknown graphics file type")]
    UnknownType,
    /// The file is damaged.
    #[error("cannot read image: {0}")]
    Corrupt(String),
    /// The requested page does not exist.
    #[error("required page does not exist ({0} pages)")]
    NoPage(i32),
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn read_png(data: &[u8]) -> Result<Image, ImageError> {
    let corrupt = || ImageError::Corrupt("PNG".into());
    let w = be32(data, 16).ok_or_else(corrupt)? as i32;
    let h = be32(data, 20).ok_or_else(corrupt)? as i32;
    let depth = i32::from(*data.get(24).ok_or_else(corrupt)?);
    let (mut x_res, mut y_res) = (0, 0);
    let mut i = 8;
    while i + 8 <= data.len() {
        let len = be32(data, i).ok_or_else(corrupt)? as usize;
        let kind = &data[i + 4..i + 8];
        if kind == b"pHYs" && data.get(i + 16) == Some(&1) {
            let ppm_x = be32(data, i + 8).ok_or_else(corrupt)?;
            let ppm_y = be32(data, i + 12).ok_or_else(corrupt)?;
            x_res = (f64::from(ppm_x) * 0.0254 + 0.5) as i32;
            y_res = (f64::from(ppm_y) * 0.0254 + 0.5) as i32;
        }
        if kind == b"IEND" {
            break;
        }
        i += 12 + len;
    }
    Ok(Image {
        kind: ImageKind::Png,
        width: w,
        height: h,
        x_res,
        y_res,
        color_depth: depth,
        pages: 1,
        page: 1,
        rotate: 0,
        bbox: [0.0, 0.0, f64::from(w), f64::from(h)],
        data: Arc::from(data),
    })
}

fn read_jpeg(data: &[u8]) -> Result<Image, ImageError> {
    let corrupt = || ImageError::Corrupt("JPEG".into());
    let (mut x_res, mut y_res) = (0, 0);
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = usize::from(be16(data, i + 2).ok_or_else(corrupt)?);
        let seg = data.get(i + 4..i + 2 + len).ok_or_else(corrupt)?;
        match marker {
            0xE0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                let unit = seg[7];
                let xd = i32::from(u16::from_be_bytes([seg[8], seg[9]]));
                let yd = i32::from(u16::from_be_bytes([seg[10], seg[11]]));
                match unit {
                    1 => (x_res, y_res) = (xd, yd),
                    2 => (x_res, y_res) = ((f64::from(xd) * 2.54) as i32, (f64::from(yd) * 2.54) as i32),
                    _ => {}
                }
            }
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let depth = i32::from(seg[0]);
                let h = i32::from(u16::from_be_bytes([seg[1], seg[2]]));
                let w = i32::from(u16::from_be_bytes([seg[3], seg[4]]));
                return Ok(Image {
                    kind: ImageKind::Jpeg,
                    width: w,
                    height: h,
                    x_res,
                    y_res,
                    color_depth: depth,
                    pages: 1,
                    page: 1,
                    rotate: 0,
                    bbox: [0.0, 0.0, f64::from(w), f64::from(h)],
                    data: Arc::from(data),
                });
            }
            _ => {}
        }
        i += 2 + len;
    }
    Err(corrupt())
}

fn number(o: &lopdf::Object) -> Option<f64> {
    match o {
        lopdf::Object::Integer(i) => Some(*i as f64),
        lopdf::Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

fn inherited<'a>(doc: &'a lopdf::Document, page: lopdf::ObjectId, key: &[u8]) -> Option<&'a lopdf::Object> {
    let mut id = page;
    for _ in 0..32 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(v) = dict.get(key) {
            return Some(match v {
                lopdf::Object::Reference(r) => doc.get_object(*r).ok()?,
                o => o,
            });
        }
        id = dict.get(b"Parent").and_then(lopdf::Object::as_reference).ok()?;
    }
    None
}

fn rect(doc: &lopdf::Document, page: lopdf::ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    let a = inherited(doc, page, key)?.as_array().ok()?;
    let v: Vec<f64> = a.iter().filter_map(number).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// The page box of a PDF page, as pdfTeX chooses it: the requested box, else the crop box, else the media box.
pub fn page_box(doc: &lopdf::Document, page: lopdf::ObjectId, which: PageBox) -> [f64; 4] {
    let media = rect(doc, page, b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let crop = rect(doc, page, b"CropBox").unwrap_or(media);
    let pick = |key: &[u8]| rect(doc, page, key).unwrap_or(crop);
    match which {
        PageBox::Media => media,
        PageBox::Crop => crop,
        PageBox::Bleed => pick(b"BleedBox"),
        PageBox::Trim => pick(b"TrimBox"),
        PageBox::Art => pick(b"ArtBox"),
    }
}

fn bp_to_sp(v: f64) -> i32 {
    (v * 65781.76).round() as i32
}

fn read_pdf(data: &[u8], page: i32, which: PageBox) -> Result<Image, ImageError> {
    let doc = lopdf::Document::load_mem(data).map_err(|e| ImageError::Corrupt(e.to_string()))?;
    let pages = doc.get_pages();
    let count = pages.len() as i32;
    let page = page.max(1);
    let id = *pages.get(&(page as u32)).ok_or(ImageError::NoPage(count))?;
    let bbox = page_box(&doc, id, which);
    let rotate = inherited(&doc, id, b"Rotate")
        .and_then(number)
        .map_or(0, |r| (r as i32).rem_euclid(360));
    Ok(Image {
        kind: ImageKind::Pdf,
        width: bp_to_sp(bbox[2] - bbox[0]),
        height: bp_to_sp(bbox[3] - bbox[1]),
        x_res: 0,
        y_res: 0,
        color_depth: 0,
        pages: count,
        page,
        rotate,
        bbox,
        data: Arc::from(data),
    })
}

/// Reads an image file, choosing the format from its signature.
pub fn read_image(data: &[u8], page: i32, which: PageBox) -> Result<Image, ImageError> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        read_png(data)
    } else if data.starts_with(&[0xFF, 0xD8]) {
        read_jpeg(data)
    } else if data.starts_with(b"%PDF") {
        read_pdf(data, page, which)
    } else {
        Err(ImageError::UnknownType)
    }
}

/// pdfTeX's `ext_xn_over_d`: `x*n/d` in floating point, rounded half away from zero.
fn ext_xn_over_d(x: Scaled, n: i32, d: i32) -> Scaled {
    if d == 0 {
        return 0;
    }
    let r = f64::from(x) * f64::from(n) / f64::from(d);
    (if r > f64::EPSILON { r + 0.5 } else { r - 0.5 }) as Scaled
}

/// `scale_image` (pdfTeX): the width, height and depth of an image given the requested ones
/// (`RUNNING` when not given) and `\pdfimageresolution`.
pub fn scale_image(
    image: &Image,
    mut width: Scaled,
    mut height: Scaled,
    mut depth: Scaled,
    image_resolution: i32,
    running: Scaled,
) -> (Scaled, Scaled, Scaled) {
    let (x, y, mut xr, mut yr) = if image.rotate == 90 || image.rotate == 270 {
        (image.height, image.width, image.y_res, image.x_res)
    } else {
        (image.width, image.height, image.x_res, image.y_res)
    };
    if xr > 65535 || yr > 65535 {
        xr = 0;
        yr = 0;
    }
    let one_inch = 4_736_287;
    if (xr != 0 || yr != 0) && (x / one_inch >= xr || y / one_inch >= yr) {
        xr = 0;
        yr = 0;
    }
    let one_hundred_inch = 473_628_672;
    let (w, h) = if image.kind == ImageKind::Pdf {
        (x, y)
    } else {
        let default_res = image_resolution.clamp(0, 65535);
        if default_res > 0 && (xr == 0 || yr == 0) {
            xr = default_res;
            yr = default_res;
        }
        if xr > 0 && yr > 0 {
            (
                ext_xn_over_d(one_hundred_inch, x, 100 * xr),
                ext_xn_over_d(one_hundred_inch, y, 100 * yr),
            )
        } else {
            (
                ext_xn_over_d(one_hundred_inch, x, 7200),
                ext_xn_over_d(one_hundred_inch, y, 7200),
            )
        }
    };
    let r = |v: Scaled| v == running;
    if r(width) && r(height) && r(depth) {
        return (w, h, 0);
    }
    if r(width) {
        if r(height) {
            width = ext_xn_over_d(h, x, y);
            height = h - depth;
        } else if r(depth) {
            width = ext_xn_over_d(height, x, y);
            depth = 0;
        } else {
            width = ext_xn_over_d(height + depth, x, y);
        }
    } else if r(height) && r(depth) {
        height = ext_xn_over_d(width, y, x);
        depth = 0;
    } else if r(height) {
        height = ext_xn_over_d(width, y, x) - depth;
    } else if r(depth) {
        depth = 0;
    }
    (width, height, depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, ppm: Option<u32>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            if let Some(p) = ppm {
                enc.set_pixel_dims(Some(png::PixelDimensions {
                    xppu: p,
                    yppu: p,
                    unit: png::Unit::Meter,
                }));
            }
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&vec![128; (w * h) as usize]).unwrap();
        }
        out
    }

    #[test]
    fn png_size_and_resolution() {
        let img = read_image(&png(30, 20, Some(3780)), 1, PageBox::Crop).unwrap();
        assert_eq!(
            (img.kind, img.width, img.height, img.color_depth),
            (ImageKind::Png, 30, 20, 8)
        );
        assert_eq!((img.x_res, img.y_res), (96, 96));
    }

    #[test]
    fn scaling_follows_pdftex() {
        let img = read_image(&png(72, 36, None), 1, PageBox::Crop).unwrap();
        let running = super::super::tex::node::RUNNING;
        let (w, h, d) = scale_image(&img, running, running, running, 0, running);
        assert_eq!((w, h, d), (4_736_287, 2_368_143, 0));
        let (w, h, _) = scale_image(&img, 2 * 65536 * 72, running, running, 0, running);
        assert_eq!((w, h), (2 * 65536 * 72, 65536 * 72));
    }

    #[test]
    fn unknown_files_are_rejected() {
        assert_eq!(read_image(b"GIF89a", 1, PageBox::Crop), Err(ImageError::UnknownType));
    }
}
