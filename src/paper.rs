//! Paper sizes and the page geometry derived from them.

use std::fmt;
use std::str::FromStr;

use crate::units::{Scaled, inch_tenths, mm};

/// A supported paper size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    /// ISO A4, 210 × 297 mm.
    #[default]
    A4,
    /// US Letter, 8.5 × 11 in.
    Letter,
}

/// The size of a page in scaled points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSize {
    /// Width of the media box.
    pub width: Scaled,
    /// Height of the media box.
    pub height: Scaled,
}

impl Paper {
    /// Every supported paper size.
    pub const ALL: [Paper; 2] = [Paper::A4, Paper::Letter];

    /// The media size.
    pub fn size(self) -> PageSize {
        match self {
            Paper::A4 => PageSize {
                width: mm(210),
                height: mm(297),
            },
            Paper::Letter => PageSize {
                width: inch_tenths(85),
                height: inch_tenths(110),
            },
        }
    }

    /// The name used on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::Letter => "letter",
        }
    }
}

impl PageSize {
    /// The TeX margin: the reference point sits one inch from the top-left corner.
    pub const MARGIN: Scaled = inch_tenths(10);

    /// `\hsize` that leaves a one-inch margin on each side.
    pub fn text_width(self) -> Scaled {
        self.width - 2 * Self::MARGIN
    }

    /// `\vsize` that leaves a one-inch margin at the top and the bottom.
    pub fn text_height(self) -> Scaled {
        self.height - 2 * Self::MARGIN
    }
}

impl fmt::Display for Paper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error for a paper name that is not supported.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown paper size `{0}` (expected a4 or letter)")]
pub struct UnknownPaper(String);

impl FromStr for Paper {
    type Err = UnknownPaper;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Paper::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| UnknownPaper(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::sp_to_bp;

    #[test]
    fn a4_matches_its_size_in_big_points() {
        let size = Paper::A4.size();
        assert!((sp_to_bp(size.width) - 595.28).abs() < 0.01);
        assert!((sp_to_bp(size.height) - 841.89).abs() < 0.01);
    }

    #[test]
    fn letter_is_612_by_792() {
        let size = Paper::Letter.size();
        assert!((sp_to_bp(size.width) - 612.0).abs() < 0.01);
        assert!((sp_to_bp(size.height) - 792.0).abs() < 0.01);
    }

    #[test]
    fn parses_names_case_insensitively() {
        assert_eq!("Letter".parse::<Paper>(), Ok(Paper::Letter));
        assert_eq!("a4".parse::<Paper>(), Ok(Paper::A4));
        assert!("a5".parse::<Paper>().is_err());
    }

    #[test]
    fn text_area_leaves_inch_margins() {
        let size = Paper::Letter.size();
        assert_eq!(size.text_width(), inch_tenths(65));
        assert_eq!(size.text_height(), inch_tenths(90));
    }
}
