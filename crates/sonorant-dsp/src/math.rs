//! `log10` and `exp` for the per-column hot paths.
//!
//! The standard library forwards these to the C runtime, where one call costs 30 to
//! 50 ns on the MinGW runtime the Windows GNU build links. A hop takes thousands of them
//! (a level per display column and history bin, and an amplitude per column for the
//! brightness), which was a third of its time. These are a handful of multiply-adds
//! each, and agree with the runtime's to within a couple of units in the last place,
//! far inside the reference tolerances.

use std::f64::consts::{LN_10, LOG2_E, LOG10_2, LOG10_E};

const MANTISSA: u64 = (1 << 52) - 1;
const ONE: u64 = 0x3ff0_0000_0000_0000;
/// Mantissa bits that pick a table entry.
const TABLE_BITS: u32 = 8;
const TABLE_LEN: usize = 1 << TABLE_BITS;

/// For each of 256 equal slices of [1, 2): the reciprocal of the slice's centre, and
/// minus its natural log. Built at compile time.
static LOG_TABLE: [(f64, f64); TABLE_LEN] = log_table();

const fn log_table() -> [(f64, f64); TABLE_LEN] {
    let mut table = [(0.0, 0.0); TABLE_LEN];
    let mut j = 0;
    while j < TABLE_LEN {
        let inv = 1.0 / (1.0 + (j as f64 + 0.5) / TABLE_LEN as f64);
        // ln(inv) = 2 atanh((inv - 1) / (inv + 1)). |s| < 0.2, so forty terms are far
        // more than a double needs; this runs once, in the compiler.
        let s = (inv - 1.0) / (inv + 1.0);
        let z = s * s;
        let mut term = s;
        let mut sum = 0.0;
        let mut k = 0;
        while k < 40 {
            sum += term / (2 * k + 1) as f64;
            term *= z;
            k += 1;
        }
        table[j] = (inv, -2.0 * sum);
        j += 1;
    }
    table
}

/// `x.log10()` for a positive, finite, normal `x`; anything else goes to the standard
/// library. Within 2e-15 of the runtime's answer.
///
/// The mantissa's top eight bits pick a slice of [1, 2) whose log is tabled; what's
/// left is within 0.2% of 1, where six terms of ln(1 + r) are exact to a double.
#[inline]
pub fn log10(x: f64) -> f64 {
    if !(f64::MIN_POSITIVE..=f64::MAX).contains(&x) {
        return x.log10();
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) as i64 - 1023) as f64;
    let m = f64::from_bits((bits & MANTISSA) | ONE);
    let (inv, ln_c) = LOG_TABLE[((bits >> (52 - TABLE_BITS)) as usize) & (TABLE_LEN - 1)];
    let r = m * inv - 1.0;
    let r2 = r * r;
    // ln(1 + r) = r - r²/2 + r³/3 - r⁴/4 + r⁵/5 - r⁶/6, |r| < 1/512: r⁷/7 < 1e-19.
    let ln_1r = r + r2 * ((-0.5 + r * (1.0 / 3.0)) + r2 * ((-0.25 + r * 0.2) + r2 * (-1.0 / 6.0)));
    exp * LOG10_2 + (ln_c + ln_1r) * LOG10_E
}

/// `x.exp()`. Arguments that would overflow or come near the subnormal range go to the
/// standard library.
#[inline]
pub fn exp(x: f64) -> f64 {
    if !(x > -700.0 && x < 700.0) {
        return x.exp();
    }
    // x = k ln 2 + r with |r| ≤ ln 2 / 2. Adding 1.5 × 2^52 rounds to the nearest
    // integer without a library call (SSE2 has no rounding instruction).
    const ROUND: f64 = 6_755_399_441_055_744.0;
    let k = (x * LOG2_E + ROUND) - ROUND;
    // ln 2 in two parts, fdlibm's: the high part's low bits are zero, so k × LN2_HI is
    // exact and r keeps its low bits.
    const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
    const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
    let r = (x - k * LN2_HI) - k * LN2_LO;
    // Taylor to r¹³, which reaches 5e-18 for |r| ≤ 0.347. Estrin's scheme rather than
    // Horner's: four short chains instead of one long one.
    let r2 = r * r;
    let r4 = r2 * r2;
    let r8 = r4 * r4;
    let c = |a: f64, b: f64| a + r * b;
    let q0 = c(1.0, 1.0) + r2 * c(1.0 / 2.0, 1.0 / 6.0);
    let q1 = c(1.0 / 24.0, 1.0 / 120.0) + r2 * c(1.0 / 720.0, 1.0 / 5040.0);
    let q2 = c(1.0 / 40320.0, 1.0 / 362_880.0) + r2 * c(1.0 / 3_628_800.0, 1.0 / 39_916_800.0);
    let q3 = c(1.0 / 479_001_600.0, 1.0 / 6_227_020_800.0);
    let p = (q0 + r4 * q1) + r8 * (q2 + r4 * q3);
    p * f64::from_bits(((k as i64 + 1023) as u64) << 52)
}

/// `10^(db / 20)`: a level in dB as a linear amplitude.
#[inline]
pub fn db_to_amplitude(db: f64) -> f64 {
    exp(db * (LN_10 / 20.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::LN_2;

    fn ulps(a: f64, b: f64) -> u64 {
        (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs()
    }

    #[test]
    fn log10_matches_the_runtime() {
        let mut worst = 0.0f64;
        let mut x: f64 = 1e-300;
        while x < 1e300 {
            for &v in &[x, x * 1.000_001, x * 1.25, x * 1.998_047, x * 1.999_999] {
                let std = v.log10();
                worst = worst.max((log10(v) - std).abs() / std.abs().max(1.0));
            }
            x *= 1.37;
        }
        // Relative, except near 1 where the answer nears 0: there it's absolute, which is
        // what a level in dB sees (2e-15 is 4e-14 dB).
        assert!(worst < 2e-15, "{worst}");
        let mut worst = 0.0f64;
        let mut x = 1e-13;
        while x < 1e3 {
            worst = worst.max((log10(x) - x.log10()).abs());
            x *= 1.000_37;
        }
        assert!(worst < 2e-15, "{worst}");
        assert_eq!(log10(1.0), 0.0);
        assert!((log10(1000.0) - 3.0).abs() < 1e-15);
        assert!(log10(0.0) == f64::NEG_INFINITY && log10(-1.0).is_nan());
    }

    #[test]
    fn exp_matches_the_runtime() {
        let mut worst = 0;
        let mut x = -699.0;
        while x < 699.0 {
            worst = worst.max(ulps(exp(x), x.exp()));
            x += LN_2 / 7.3;
        }
        for i in 0..20000 {
            let x = -40.0 + i as f64 * 0.0023;
            worst = worst.max(ulps(exp(x), x.exp()));
        }
        assert!(worst <= 4, "{worst} ulps");
        assert_eq!(exp(0.0), 1.0);
        assert!(exp(-800.0) == 0.0 && exp(800.0).is_infinite() && exp(f64::NAN).is_nan());
        assert!((db_to_amplitude(-20.0) - 0.1).abs() < 1e-16);
    }
}
