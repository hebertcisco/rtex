//! pdfTeX's random numbers (`\pdfuniformdeviate`, `\pdfnormaldeviate`), MetaPost's generator.

const FRACTION_HALF: i32 = 0o1000000000;
const FRACTION_ONE: i32 = 0o2000000000;
const FRACTION_FOUR: i32 = 0o10000000000;
const EL_GORDO: i32 = 0o17777777777;
const UNITY: i32 = 0o200000;

const SPEC_LOG: [i32; 29] = [
    0, 93_032_640, 38_612_034, 17_922_280, 8_662_214, 4_261_238, 2_113_709, 1_052_693, 525_315, 262_400, 131_136,
    65_552, 32_772, 16_385, 8192, 4096, 2048, 1024, 512, 256, 128, 64, 32, 16, 8, 4, 2, 1, 1,
];

/// The state of the generator: the last 55 values and how many are unused.
#[derive(Clone, Debug)]
pub(crate) struct Random {
    randoms: [i32; 55],
    j: usize,
}

impl Default for Random {
    fn default() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Random::new((secs % 1_000_000) as i32)
    }
}

fn make_frac(mut p: i32, mut q: i32) -> i32 {
    let mut negative = p < 0;
    if negative {
        p = -p;
    }
    if q <= 0 {
        q = -q;
        negative = !negative;
    }
    if q == 0 {
        return 0;
    }
    let n = p / q;
    p %= q;
    if n >= 8 {
        return if negative { -EL_GORDO } else { EL_GORDO };
    }
    let n = (n - 1) * FRACTION_ONE;
    let mut f = 1;
    loop {
        let be_careful = p - q;
        p += be_careful;
        if p >= 0 {
            f = f + f + 1;
        } else {
            f += f;
            p += q;
        }
        if f >= FRACTION_ONE {
            break;
        }
    }
    if p - q + p >= 0 {
        f += 1;
    }
    if negative { -(f + n) } else { f + n }
}

fn take_frac(mut q: i32, mut f: i32) -> i32 {
    let mut negative = f < 0;
    if negative {
        f = -f;
    }
    if q < 0 {
        q = -q;
        negative = !negative;
    }
    let mut n = if f < FRACTION_ONE {
        0
    } else {
        let k = f / FRACTION_ONE;
        f %= FRACTION_ONE;
        if q <= EL_GORDO / k { k * q } else { EL_GORDO }
    };
    f += FRACTION_ONE;
    let mut p = FRACTION_HALF;
    if q < FRACTION_FOUR {
        loop {
            p = if f % 2 == 1 { (p + q) / 2 } else { p / 2 };
            f /= 2;
            if f == 1 {
                break;
            }
        }
    } else {
        loop {
            p = if f % 2 == 1 { p + (q - p) / 2 } else { p / 2 };
            f /= 2;
            if f == 1 {
                break;
            }
        }
    }
    if n - EL_GORDO + p > 0 {
        n = EL_GORDO - p;
    }
    if negative { -(n + p) } else { n + p }
}

fn m_log(mut x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }
    let mut y: i32 = 1_302_456_956 + 4 - 100;
    let mut z: i32 = 27_595 + 6_553_600;
    while x < FRACTION_FOUR {
        x += x;
        y -= 93_032_639;
        z -= 48_782;
    }
    y += z / UNITY;
    let mut k = 2;
    while x > FRACTION_FOUR + 4 {
        let mut z = ((x - 1) / (1 << k)) + 1;
        while x < FRACTION_FOUR + z {
            z = (z + 1) / 2;
            k += 1;
        }
        y += SPEC_LOG[k];
        x -= z;
    }
    y / 8
}

fn ab_vs_cd(mut a: i32, mut b: i32, mut c: i32, mut d: i32) -> i32 {
    if a < 0 {
        a = -a;
        b = -b;
    }
    if c < 0 {
        c = -c;
        d = -d;
    }
    if d <= 0 {
        if b >= 0 {
            return if (a == 0 || b == 0) && (c == 0 || d == 0) { 0 } else { 1 };
        }
        if d == 0 {
            return if a == 0 { 0 } else { -1 };
        }
        std::mem::swap(&mut a, &mut c);
        let q = -b;
        b = -d;
        d = q;
    } else if b <= 0 {
        if b < 0 && a > 0 {
            return -1;
        }
        return if c == 0 { 0 } else { -1 };
    }
    loop {
        let q = a / d;
        let r = c / b;
        if q != r {
            return if q > r { 1 } else { -1 };
        }
        let q = a % d;
        let r = c % b;
        if r == 0 {
            return if q == 0 { 0 } else { 1 };
        }
        if q == 0 {
            return -1;
        }
        a = b;
        b = q;
        c = d;
        d = r;
    }
}

impl Random {
    /// `init_randoms`.
    pub fn new(seed: i32) -> Self {
        let mut r = Random { randoms: [0; 55], j: 0 };
        let mut j = seed.wrapping_abs();
        while j >= FRACTION_ONE {
            j /= 2;
        }
        let mut k = 1;
        for i in 0..55 {
            let jj = k;
            k = j - k;
            j = jj;
            if k < 0 {
                k += FRACTION_ONE;
            }
            r.randoms[(i * 21) % 55] = j;
        }
        r.new_randoms();
        r.new_randoms();
        r.new_randoms();
        r
    }

    fn new_randoms(&mut self) {
        for k in 0..24 {
            let mut x = self.randoms[k] - self.randoms[k + 31];
            if x < 0 {
                x += FRACTION_ONE;
            }
            self.randoms[k] = x;
        }
        for k in 24..55 {
            let mut x = self.randoms[k] - self.randoms[k - 24];
            if x < 0 {
                x += FRACTION_ONE;
            }
            self.randoms[k] = x;
        }
        self.j = 54;
    }

    fn next(&mut self) -> i32 {
        if self.j == 0 {
            self.new_randoms();
        } else {
            self.j -= 1;
        }
        self.randoms[self.j]
    }

    /// `unif_rand`: a uniform deviate between 0 and `x`.
    pub fn unif_rand(&mut self, x: i32) -> i32 {
        let r = self.next();
        let y = take_frac(x.wrapping_abs(), r);
        if y == x.wrapping_abs() {
            0
        } else if x > 0 {
            y
        } else {
            -y
        }
    }

    /// `norm_rand`: a normal deviate with mean 0 and deviation 65536.
    pub fn norm_rand(&mut self) -> i32 {
        loop {
            let (x, u) = loop {
                let r = self.next();
                let x = take_frac(112_429, r - FRACTION_HALF);
                let u = self.next();
                if x.abs() < u {
                    break (x, u);
                }
            };
            let x = make_frac(x, u);
            let l = 139_548_960 - m_log(u);
            if ab_vs_cd(1024, l, x, x) >= 0 {
                return x;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_deviates_stay_in_range_and_repeat() {
        let mut a = Random::new(12345);
        let mut b = Random::new(12345);
        for _ in 0..100 {
            let v = a.unif_rand(1000);
            assert!((0..1000).contains(&v));
            assert_eq!(v, b.unif_rand(1000));
        }
        let n: i64 = (0..1000).map(|_| i64::from(a.norm_rand())).sum();
        assert!((n / 1000).abs() < 20_000);
    }
}
