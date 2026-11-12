//! TeX's scaled points and the conversions the typesetter and the PDF writer need.

pub use rtex_core::{Scaled, UNITY};

#[cfg(test)]
const SP_PER_BP: f64 = 65_536.0 * 72.27 / 72.0;

/// `n` printer's points.
#[cfg(test)]
pub const fn pt(n: i32) -> Scaled {
    n * UNITY
}

/// `tenths / 10` inches, as TeX rounds `1in` (4736286sp).
pub const fn inch_tenths(tenths: i64) -> Scaled {
    (tenths * 4_736_286 / 10) as Scaled
}

/// `n` millimetres.
pub const fn mm(n: i64) -> Scaled {
    (n * 7227 * UNITY as i64 / 2540) as Scaled
}

/// Scales a quantity given in thousandths of `size`, rounding to the nearest scaled point.
#[cfg(test)]
pub fn per_mille(units: i32, size: Scaled) -> Scaled {
    let product = i64::from(units) * i64::from(size);
    ((product + product.signum() * 500) / 1000) as Scaled
}

/// Converts scaled points to PDF units (big points).
#[cfg(test)]
pub fn sp_to_bp(sp: Scaled) -> f32 {
    (f64::from(sp) / SP_PER_BP) as f32
}

/// Formats a dimension the way TeX's `print_scaled` does, without the unit (§103).
#[cfg(test)]
pub fn format_scaled(s: Scaled) -> String {
    let mut out = String::new();
    let mut s = i64::from(s);
    if s < 0 {
        out.push('-');
        s = -s;
    }
    let unity = i64::from(UNITY);
    out.push_str(&(s / unity).to_string());
    out.push('.');
    s = 10 * (s % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            s += 0x8000 - 50_000;
        }
        out.push(char::from(b'0' + (s / unity) as u8));
        s = 10 * (s % unity);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_inch_is_72_27_points() {
        assert_eq!(format_scaled(inch_tenths(10)), "72.26999");
    }

    #[test]
    fn prints_like_tex() {
        assert_eq!(format_scaled(pt(10)), "10.0");
        assert_eq!(format_scaled(UNITY / 2), "0.5");
        assert_eq!(format_scaled(-UNITY * 3 / 2), "-1.5");
        assert_eq!(format_scaled(1), "0.00002");
    }

    #[test]
    fn converts_to_big_points() {
        assert!((sp_to_bp(inch_tenths(10)) - 72.0).abs() < 1e-3);
    }

    #[test]
    fn scales_per_mille_with_rounding() {
        assert_eq!(per_mille(250, pt(10)), pt(10) / 4);
        assert_eq!(per_mille(-250, pt(10)), -pt(10) / 4);
    }
}
