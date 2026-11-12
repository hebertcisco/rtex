//! TeX font metric files, read and scaled exactly as `read_font_info` does (§539–§576).

use rtex_core::xn_over_d;

use crate::units::Scaled;

/// Why a TFM file was rejected (TeX's "Bad metric (TFM) file").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadTfm;

/// What a character's `tag` field says about it (§544).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// Nothing special.
    None,
    /// It has a ligature/kern program starting at the remainder.
    LigKern(u16),
    /// The next larger character in a chain of sizes.
    List(u8),
    /// An extensible character described by recipe `remainder`.
    Extensible(u8),
}

/// One instruction of a ligature/kern program (§545).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LigKernStep {
    /// `skip_byte`: 0 continues with the next instruction, ≥ 128 stops.
    pub skip: u8,
    /// The character this step applies to.
    pub next_char: u8,
    /// `op_byte`: ≥ 128 is a kern, otherwise a ligature operation.
    pub op: u8,
    /// `rem_byte`: the kern index or the ligature character.
    pub remainder: u8,
}

impl LigKernStep {
    /// Whether this step is a kern.
    pub fn is_kern(&self) -> bool {
        self.op >= 128
    }

    /// The index of the kern for a kern step.
    pub fn kern_index(&self) -> usize {
        256 * usize::from(self.op - 128) + usize::from(self.remainder)
    }
}

/// The pieces of an extensible character (§546).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extensible {
    /// Top piece, 0 if absent.
    pub top: u8,
    /// Middle piece, 0 if absent.
    pub mid: u8,
    /// Bottom piece, 0 if absent.
    pub bot: u8,
    /// Repeated piece.
    pub rep: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct CharInfo {
    width: u8,
    height: u8,
    depth: u8,
    italic: u8,
    tag: u8,
    remainder: u8,
}

/// A font loaded at a size.
#[derive(Clone, Debug)]
pub struct Tfm {
    /// The checksum from the header.
    pub checksum: u32,
    /// The design size.
    pub design_size: Scaled,
    /// The size at which it is used.
    pub size: Scaled,
    bc: u8,
    ec: u8,
    chars: Vec<CharInfo>,
    widths: Vec<Scaled>,
    heights: Vec<Scaled>,
    depths: Vec<Scaled>,
    italics: Vec<Scaled>,
    lig_kern: Vec<LigKernStep>,
    kerns: Vec<Scaled>,
    exten: Vec<Extensible>,
    params: Vec<Scaled>,
    bchar_label: Option<usize>,
    font_bchar: Option<u8>,
}

struct Bytes<'a> {
    data: &'a [u8],
}

impl Bytes<'_> {
    fn word(&self, i: usize) -> Result<[u8; 4], BadTfm> {
        let s = self.data.get(4 * i..4 * i + 4).ok_or(BadTfm)?;
        Ok([s[0], s[1], s[2], s[3]])
    }

    fn halves(&self, i: usize) -> Result<(usize, usize), BadTfm> {
        let w = self.word(i)?;
        if w[0] > 127 || w[2] > 127 {
            return Err(BadTfm);
        }
        Ok((
            usize::from(w[0]) * 256 + usize::from(w[1]),
            usize::from(w[2]) * 256 + usize::from(w[3]),
        ))
    }
}

struct Scaler {
    z: i32,
    alpha: i32,
    beta: i32,
}

impl Scaler {
    fn new(size: Scaled) -> Self {
        let mut z = size;
        let mut alpha = 16;
        while z >= 0o40000000 {
            z /= 2;
            alpha += alpha;
        }
        let beta = 256 / alpha;
        Scaler {
            z,
            alpha: alpha * z,
            beta,
        }
    }

    fn store_scaled(&self, w: [u8; 4]) -> Result<Scaled, BadTfm> {
        let [a, b, c, d] = w.map(i32::from);
        let z = self.z;
        let sw = (((d * z) / 256 + c * z) / 256 + b * z) / self.beta;
        match a {
            0 => Ok(sw),
            255 => Ok(sw - self.alpha),
            _ => Err(BadTfm),
        }
    }
}

impl Tfm {
    /// Parses `data` and scales it: `size > 0` is `at size`, `size = -1000·m` is `scaled m` (§560).
    pub fn parse(data: &[u8], size: i32) -> Result<Tfm, BadTfm> {
        let b = Bytes { data };
        let (lf, lh) = b.halves(0)?;
        let (bc, ec) = b.halves(1)?;
        let (nw, nh) = b.halves(2)?;
        let (nd, ni) = b.halves(3)?;
        let (nl, nk) = b.halves(4)?;
        let (ne, np) = b.halves(5)?;
        let (bc, ec) = if bc > ec + 1 || ec > 255 {
            return Err(BadTfm);
        } else if bc > 255 {
            (1, 0)
        } else {
            (bc, ec)
        };
        if lf != 6 + lh + (ec + 1 - bc) + nw + nh + nd + ni + nl + nk + ne + np
            || nw == 0
            || nh == 0
            || nd == 0
            || ni == 0
            || lh < 2
            || data.len() < 4 * lf
        {
            return Err(BadTfm);
        }
        let header = 6;
        let checksum = u32::from_be_bytes(b.word(header)?);
        let ds = b.word(header + 1)?;
        if ds[0] > 127 {
            return Err(BadTfm);
        }
        let ds = ((i32::from(ds[0]) * 256 + i32::from(ds[1])) * 256 + i32::from(ds[2])) * 16 + i32::from(ds[3]) / 16;
        if ds < 65536 {
            return Err(BadTfm);
        }
        let design_size = ds;
        let size = if size == -1000 {
            design_size
        } else if size < 0 {
            xn_over_d(design_size, -size, 1000).quotient
        } else {
            size
        };
        let char_base = header + lh;
        let width_base = char_base + (ec + 1 - bc);
        let height_base = width_base + nw;
        let depth_base = height_base + nh;
        let italic_base = depth_base + nd;
        let lig_base = italic_base + ni;
        let kern_base = lig_base + nl;
        let exten_base = kern_base + nk;
        let param_base = exten_base + ne;

        let mut chars = Vec::with_capacity(ec + 1 - bc);
        for k in 0..(ec + 1 - bc) {
            let w = b.word(char_base + k)?;
            let info = CharInfo {
                width: w[0],
                height: w[1] / 16,
                depth: w[1] % 16,
                italic: w[2] / 4,
                tag: w[2] % 4,
                remainder: w[3],
            };
            if usize::from(info.width) >= nw
                || usize::from(info.height) >= nh
                || usize::from(info.depth) >= nd
                || usize::from(info.italic) >= ni
            {
                return Err(BadTfm);
            }
            chars.push(info);
        }
        let scaler = Scaler::new(size);
        let scaled = |base: usize, n: usize| -> Result<Vec<Scaled>, BadTfm> {
            (0..n).map(|i| scaler.store_scaled(b.word(base + i)?)).collect()
        };
        let widths = scaled(width_base, nw)?;
        let heights = scaled(height_base, nh)?;
        let depths = scaled(depth_base, nd)?;
        let italics = scaled(italic_base, ni)?;
        if widths[0] != 0 || heights[0] != 0 || depths[0] != 0 || italics[0] != 0 {
            return Err(BadTfm);
        }
        let lig_kern: Vec<LigKernStep> = (0..nl)
            .map(|i| {
                b.word(lig_base + i).map(|w| LigKernStep {
                    skip: w[0],
                    next_char: w[1],
                    op: w[2],
                    remainder: w[3],
                })
            })
            .collect::<Result<_, _>>()?;
        let kerns = scaled(kern_base, nk)?;
        let exten: Vec<Extensible> = (0..ne)
            .map(|i| {
                b.word(exten_base + i).map(|w| Extensible {
                    top: w[0],
                    mid: w[1],
                    bot: w[2],
                    rep: w[3],
                })
            })
            .collect::<Result<_, _>>()?;
        let mut params = Vec::with_capacity(np.max(7));
        for k in 0..np {
            let w = b.word(param_base + k)?;
            if k == 0 {
                let mut sw = i32::from(w[0]);
                if sw > 127 {
                    sw -= 256;
                }
                sw = sw * 256 + i32::from(w[1]);
                sw = sw * 256 + i32::from(w[2]);
                params.push(sw * 16 + i32::from(w[3]) / 16);
            } else {
                params.push(scaler.store_scaled(w)?);
            }
        }
        while params.len() < 7 {
            params.push(0);
        }
        let mut bchar_label = None;
        let mut font_bchar = None;
        if let Some(first) = lig_kern.first()
            && first.skip == 255
        {
            font_bchar = Some(first.next_char);
        }
        if let Some(last) = lig_kern.last()
            && last.skip == 255
        {
            bchar_label = Some(256 * usize::from(last.op) + usize::from(last.remainder));
        }
        Ok(Tfm {
            checksum,
            design_size,
            size,
            bc: bc as u8,
            ec: ec as u8,
            chars,
            widths,
            heights,
            depths,
            italics,
            lig_kern,
            kerns,
            exten,
            params,
            bchar_label,
            font_bchar,
        })
    }

    fn info(&self, c: u32) -> Option<&CharInfo> {
        let c = u8::try_from(c).ok()?;
        if c < self.bc || c > self.ec {
            return None;
        }
        let info = &self.chars[usize::from(c - self.bc)];
        (info.width > 0).then_some(info)
    }

    /// Whether the font has character `c` (`char_exists`).
    pub fn has_char(&self, c: u32) -> bool {
        self.info(c).is_some()
    }

    /// The width of `c`, zero if absent.
    pub fn width(&self, c: u32) -> Scaled {
        self.info(c).map_or(0, |i| self.widths[usize::from(i.width)])
    }

    /// The height of `c`.
    pub fn height(&self, c: u32) -> Scaled {
        self.info(c).map_or(0, |i| self.heights[usize::from(i.height)])
    }

    /// The depth of `c`.
    pub fn depth(&self, c: u32) -> Scaled {
        self.info(c).map_or(0, |i| self.depths[usize::from(i.depth)])
    }

    /// The italic correction of `c`.
    pub fn italic(&self, c: u32) -> Scaled {
        self.info(c).map_or(0, |i| self.italics[usize::from(i.italic)])
    }

    /// The tag of `c`.
    pub fn tag(&self, c: u32) -> Tag {
        match self.info(c) {
            Some(i) => match i.tag {
                1 => Tag::LigKern(u16::from(i.remainder)),
                2 => Tag::List(i.remainder),
                3 => Tag::Extensible(i.remainder),
                _ => Tag::None,
            },
            None => Tag::None,
        }
    }

    /// The first step of the ligature/kern program at `remainder`, following TeX's indirection (§557).
    pub fn lig_kern_start(&self, remainder: usize) -> Option<usize> {
        let step = self.lig_kern.get(remainder)?;
        if step.skip > 128 {
            Some(256 * usize::from(step.op) + usize::from(step.remainder))
        } else {
            Some(remainder)
        }
    }

    /// The program of `c`, if it has one.
    pub fn program_of(&self, c: u32) -> Option<usize> {
        match self.tag(c) {
            Tag::LigKern(r) => self.lig_kern_start(usize::from(r)),
            _ => None,
        }
    }

    /// The step at index `i`.
    pub fn step(&self, i: usize) -> Option<LigKernStep> {
        self.lig_kern.get(i).copied()
    }

    /// The kern of a kern step.
    pub fn kern(&self, step: &LigKernStep) -> Scaled {
        self.kerns.get(step.kern_index()).copied().unwrap_or(0)
    }

    /// Where the boundary-character program starts (`bchar_label`).
    pub fn bchar_label(&self) -> Option<usize> {
        self.bchar_label
    }

    /// The right boundary character, if the font names one (`font_bchar`).
    pub fn boundary_char(&self) -> Option<u8> {
        self.font_bchar
    }

    /// `font_false_bchar` (§576): the boundary character unless it is a real character, else 256.
    pub fn false_boundary_char(&self) -> u32 {
        match self.font_bchar {
            Some(b) if !self.has_char(u32::from(b)) => u32::from(b),
            _ => 256,
        }
    }

    /// The extensible recipe `i`.
    pub fn extensible(&self, i: u8) -> Option<Extensible> {
        self.exten.get(usize::from(i)).copied()
    }

    /// The `\fontdimen` parameters, 0-based, at least seven.
    pub fn params(&self) -> &[Scaled] {
        &self.params
    }

    /// The smallest and largest character codes.
    pub fn char_range(&self) -> (u8, u8) {
        (self.bc, self.ec)
    }

    /// Returns a copy without the ligature/kern program: every character with a
    /// ligature/kern tag is retagged `None` (`set_no_ligatures`, pdfTeX §1354; the `nolig` keyword
    /// of `\letterspacefont`).
    pub fn without_lig_kern(&self) -> Self {
        let mut copy = self.clone();
        for info in copy.chars.iter_mut() {
            if info.tag == 1 {
                info.tag = 0;
            }
        }
        copy
    }

    /// Returns a copy with character widths increased for letterspacing
    /// (`\letterspacefont`), adding `round(quad * e / 1000)` to each width (round half away from zero).
    pub fn letterspaced(&self, e: i32) -> Self {
        let mut copy = self.clone();
        let quad = self.params.get(5).copied().unwrap_or(0);
        let t = i64::from(quad) * i64::from(e);
        let delta = (if t >= 0 { (t + 500) / 1000 } else { (t - 500) / 1000 }) as i32;
        for (i, w) in copy.widths.iter_mut().enumerate() {
            if i != 0 {
                *w += delta;
            }
        }
        copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kpse::{FileKind, Kpathsea, Program};

    fn cmr10(size: i32) -> Option<Tfm> {
        let mut kpse = Kpathsea::discover(Program::PlainTex);
        let path = kpse.find("cmr10", FileKind::Tfm)?;
        Some(Tfm::parse(&std::fs::read(path).ok()?, size).expect("cmr10 parses"))
    }

    #[test]
    fn cmr10_metrics_match_tex() {
        let Some(f) = cmr10(-1000) else { return };
        assert_eq!(f.design_size, 10 * 65536);
        assert_eq!(f.width(u32::from('A')), 491521);
        assert_eq!(f.height(u32::from('A')), 447828);
        assert_eq!(f.width(u32::from('x')), 345886);
        assert_eq!(f.params()[1], 218453);
        assert!(f.program_of(u32::from('f')).is_some());
        assert!(!f.has_char(200));
    }

    #[test]
    fn scaled_fonts_scale_every_dimension() {
        let Some(f) = cmr10(-2000) else { return };
        assert_eq!(f.size, 20 * 65536);
        assert_eq!(f.width(u32::from('x')), 691772);
    }

    #[test]
    fn letterspaced_uses_round_half_away_from_zero() {
        let Some(f) = cmr10(-1000) else { return };
        let wider = f.letterspaced(100);
        let quad = f.params()[5];
        let delta = ((i64::from(quad) * 100 + 500) / 1000) as i32;
        assert_eq!(wider.width(u32::from('x')), f.width(u32::from('x')) + delta);
        let narrower = f.letterspaced(-100);
        assert_eq!(narrower.width(u32::from('x')), f.width(u32::from('x')) - delta);
    }

    #[test]
    fn without_lig_kern_clears_ligature_programs() {
        let Some(f) = cmr10(-1000) else { return };
        assert!(f.program_of(u32::from('f')).is_some());
        let stripped = f.without_lig_kern();
        assert!(stripped.program_of(u32::from('f')).is_none());
        assert_eq!(stripped.tag(u32::from('f')), Tag::None);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(Tfm::parse(&[0; 16], -1000).err(), Some(BadTfm));
        assert_eq!(Tfm::parse(&[], -1000).err(), Some(BadTfm));
    }
}
