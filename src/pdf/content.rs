//! Page content streams, positioned as pdfTeX positions them: a `set_origin` literal is
//! wrapped in `cm` moves to the current point and back, so the matrix it sets applies around
//! that point to what follows.

use std::collections::{BTreeMap, BTreeSet};

use super::file::{bp, real};
use crate::document::{Document, DrawOp, FontId, PdfOp};
use crate::tex::backend::LiteralMode;
use crate::units::Scaled;

/// The fonts and XObjects a content stream uses.
#[derive(Clone, Debug, Default)]
pub(crate) struct Resources {
    pub fonts: BTreeSet<FontId>,
    pub forms: BTreeSet<usize>,
    pub images: BTreeSet<usize>,
}

/// Slant and extension of a font as the map file gives them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FontShape {
    pub slant: f64,
    pub extend: f64,
}

struct Builder<'a> {
    out: Vec<u8>,
    digits: usize,
    height: Scaled,
    origin: (f64, f64),
    in_text: bool,
    font: Option<FontId>,
    document: &'a Document,
    shapes: &'a BTreeMap<FontId, FontShape>,
    resources: Resources,
}

impl Builder<'_> {
    fn num(&self, v: f64) -> String {
        real(v, self.digits)
    }

    fn line(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
        self.out.push(b'\n');
    }

    fn begin_text(&mut self) {
        if !self.in_text {
            self.line("BT");
            self.in_text = true;
            self.font = None;
        }
    }

    fn end_text(&mut self) {
        if self.in_text {
            self.line("ET");
            self.in_text = false;
        }
    }

    fn xy(&self, h: Scaled, v: Scaled) -> (f64, f64) {
        (bp(h) - self.origin.0, bp(self.height - v) - self.origin.1)
    }

    fn set_origin(&mut self, h: Scaled, v: Scaled) {
        let (dx, dy) = self.xy(h, v);
        let k = 10f64.powi(self.digits as i32);
        let (rx, ry) = ((dx * k).round() / k, (dy * k).round() / k);
        if rx != 0.0 || ry != 0.0 {
            let s = format!("1 0 0 1 {} {} cm", self.num(rx), self.num(ry));
            self.line(&s);
            self.origin.0 += rx;
            self.origin.1 += ry;
        }
    }

    fn text(&mut self, font: FontId, x: Scaled, baseline: Scaled, codes: &[u32]) {
        let Some(doc) = self.document.font(font) else { return };
        self.begin_text();
        if self.font != Some(font) {
            let size = real(bp(doc.size), 4);
            self.line(&format!("/F{font} {size} Tf"));
            self.font = Some(font);
            self.resources.fonts.insert(font);
        }
        let shape = self.shapes.get(&font).copied().unwrap_or(FontShape {
            slant: 0.0,
            extend: 1.0,
        });
        let (x, y) = self.xy(x, baseline);
        let tm = format!(
            "{} 0 {} 1 {} {} Tm",
            real(shape.extend, 4),
            real(shape.slant, 4),
            self.num(x),
            self.num(y)
        );
        self.line(&tm);
        let mut s = vec![b'('];
        for &c in codes {
            let b = c.min(255) as u8;
            match b {
                b'(' | b')' | b'\\' => {
                    s.push(b'\\');
                    s.push(b);
                }
                b'\r' => s.extend_from_slice(b"\\r"),
                b'\n' => s.extend_from_slice(b"\\n"),
                _ => s.push(b),
            }
        }
        s.extend_from_slice(b") Tj");
        self.out.extend_from_slice(&s);
        self.out.push(b'\n');
    }

    fn rule(&mut self, x: Scaled, top: Scaled, width: Scaled, height: Scaled) {
        self.end_text();
        let (x, y) = self.xy(x, top + height);
        let s = format!(
            "{} {} {} {} re f",
            self.num(x),
            self.num(y),
            self.num(bp(width)),
            self.num(bp(height))
        );
        self.line(&s);
    }

    fn literal(&mut self, h: Scaled, v: Scaled, mode: LiteralMode, text: &str) {
        match mode {
            LiteralMode::SetOrigin => {
                self.end_text();
                self.set_origin(h, v);
                self.line(text);
                self.reset_origin();
            }
            LiteralMode::Page => {
                self.end_text();
                self.line(text);
            }
            LiteralMode::Direct => self.line(text),
        }
    }

    fn reset_origin(&mut self) {
        let (x, y) = self.origin;
        if x != 0.0 || y != 0.0 {
            let s = format!("1 0 0 1 {} {} cm", self.num(-x), self.num(-y));
            self.line(&s);
            self.origin = (0.0, 0.0);
        }
    }

    fn xobject(&mut self, name: &str, h: Scaled, bottom: Scaled, scale: Option<(Scaled, Scaled)>) {
        self.end_text();
        let (x, y) = self.xy(h, bottom);
        let m = match scale {
            Some((w, ht)) => format!("{} 0 0 {}", self.num(bp(w)), self.num(bp(ht))),
            None => "1 0 0 1".to_owned(),
        };
        let s = format!("q {m} {} {} cm /{name} Do Q", self.num(x), self.num(y));
        self.line(&s);
    }
}

/// The content stream of `ops` on a page of height `height`, with the resources it uses.
pub(crate) fn render(
    document: &Document,
    shapes: &BTreeMap<FontId, FontShape>,
    ops: &[DrawOp],
    height: Scaled,
    digits: usize,
    job: &super::PdfJob,
) -> (Vec<u8>, Resources) {
    let mut b = Builder {
        out: Vec::new(),
        digits,
        height,
        origin: (0.0, 0.0),
        in_text: false,
        font: None,
        document,
        shapes,
        resources: Resources::default(),
    };
    for op in ops {
        match op {
            DrawOp::Text {
                font,
                x,
                baseline,
                codes,
                ..
            } => b.text(*font, *x, *baseline, codes),
            DrawOp::Rule { x, top, width, height } => b.rule(*x, *top, *width, *height),
            DrawOp::Special { .. } => {}
            DrawOp::Pdf { x, y, op } => match op {
                PdfOp::Literal { mode, text } => b.literal(*x, *y, *mode, text),
                PdfOp::Form { obj, depth } => {
                    b.resources.forms.insert(*obj);
                    b.xobject(&super::xobject_name(job, *obj), *x, *y + *depth, None);
                }
                PdfOp::Image {
                    obj,
                    width,
                    height,
                    depth,
                } => {
                    b.resources.images.insert(*obj);
                    b.xobject(
                        &super::xobject_name(job, *obj),
                        *x,
                        *y + *depth,
                        Some((*width, *height + *depth)),
                    );
                }
                PdfOp::Annot { .. } | PdfOp::Link { .. } | PdfOp::Dest { .. } | PdfOp::Structure(_) => {}
            },
        }
    }
    b.end_text();
    (b.out, b.resources)
}
