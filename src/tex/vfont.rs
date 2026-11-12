//! Virtual fonts (`virtual_font_type` in pdfTeX): fonts whose characters are packets of
//! typesetting operations executed with the help of packet-local fonts (`vf_i_fnts`).
//! `\letterspacefont` is the only producer of virtual fonts today; the packet of every
//! character is `kern w + setchar c + kern w` with `w = round(quad*e/2000)`, executed at
//! shipout time with the base font as local font (`do_vf_packet` in pdftex.web).

use std::collections::HashMap;

use crate::units::Scaled;

/// One operation of a virtual-font character packet (`do_vf_packet` in pdftex.web).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VfOp {
    /// `right<n>`: move the pen horizontally by `n` scaled points (negative moves left).
    Kern(Scaled),
    /// `set<n> <c>`: typeset character `ch` from local font `local` (1-based index into
    /// `VirtualFont::local_fonts`), advancing the pen by the local font's width of `ch`.
    SetChar { local: u32, ch: u32 },
}

/// A virtual font created by `\letterspacefont`.
#[derive(Clone, Debug, Default)]
pub(crate) struct VirtualFont {
    /// Engine font numbers for the packet-local fonts, 1-based inside packets (`vf_i_fnts`).
    pub local_fonts: Vec<usize>,
    /// The character packet of each character (`storepacket`).
    pub packets: HashMap<u32, Vec<VfOp>>,
}

/// Rounds `x*n/d` half away from zero, matching TeX's `round_xn_over_d` (tex.web §108/§1030 area;
/// pdftex.web uses the same rounding for the packet kern).
pub(crate) fn round_xn(x: i32, n: i32, d: i32) -> Scaled {
    let t = i64::from(x) * i64::from(n);
    let d = i64::from(d);
    (if t >= 0 { (t + d / 2) / d } else { (t - d / 2) / d }) as i32
}

impl VirtualFont {
    /// Builds the letterspaced packet program for the range `bc..=ec` of the base font:
    /// `kern w + set1 c + kern w` for every code, with local font the base font itself
    /// (`vf_local_font_num[k] := 1`, `vf_i_fnts[vf_default_font[k]] := base`).
    pub(crate) fn letterspacing(source: usize, bc: u8, ec: u8, quad: Scaled, e: i32) -> Self {
        let w = round_xn(quad, e, 2000);
        let packets = (bc..=ec)
            .map(|c| {
                (
                    u32::from(c),
                    vec![
                        VfOp::Kern(w),
                        VfOp::SetChar {
                            local: 1,
                            ch: u32::from(c),
                        },
                        VfOp::Kern(w),
                    ],
                )
            })
            .collect();
        Self {
            local_fonts: vec![source],
            packets,
        }
    }
}
