//! Deterministic f64 operations shared by the VM and compiler constant folder.

/// The single quiet-NaN payload rustia uses for every script-number NaN.
pub const CANON_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    Neg,
    Abs,
}

#[inline]
pub fn canonicalize_number(value: f64) -> f64 {
    if value.is_nan() {
        f64::from_bits(CANON_NAN_BITS)
    } else {
        value
    }
}

#[inline]
pub fn table_key_number_bits(value: f64) -> u64 {
    let normalized = if value == 0.0 { 0.0 } else { value };
    canonicalize_number(normalized).to_bits()
}

#[inline]
pub fn floored_mod(lhs: f64, rhs: f64) -> f64 {
    // Integer counters and table/hash arithmetic commonly use a positive integer
    // modulus. Every integer represented by `lhs` in this range converts exactly to
    // `u64`, and every possible remainder converts exactly back to `f64`, so integer
    // remainder has the same result as `fmod` without entering libm.
    if !lhs.is_sign_negative()
        && (0.0..18_446_744_073_709_551_616.0).contains(&lhs)
        && lhs.fract() == 0.0
        && (1.0..=9_007_199_254_740_992.0).contains(&rhs)
        && rhs.fract() == 0.0
    {
        let divisor = rhs as u64;
        let dividend = lhs as u64;
        let remainder = if divisor.is_power_of_two() {
            dividend & (divisor - 1)
        } else {
            dividend % divisor
        };
        return remainder as f64;
    }
    let mut value = libm::fmod(lhs, rhs);
    if value != 0.0 && ((value < 0.0) != (rhs < 0.0)) {
        value += rhs;
    }
    canonicalize_number(value)
}

#[inline]
pub fn floor_div(lhs: f64, rhs: f64) -> f64 {
    canonicalize_number(libm::floor(lhs / rhs))
}

#[inline]
pub fn binary(op: NumericOp, lhs: f64, rhs: f64) -> f64 {
    let value = match op {
        NumericOp::Add => lhs + rhs,
        NumericOp::Sub => lhs - rhs,
        NumericOp::Mul => lhs * rhs,
        NumericOp::Div => lhs / rhs,
        NumericOp::FloorDiv => floor_div(lhs, rhs),
        NumericOp::Mod => floored_mod(lhs, rhs),
        NumericOp::Pow => libm::pow(lhs, rhs),
        NumericOp::Neg => -lhs,
        NumericOp::Abs => {
            if lhs.is_sign_negative() {
                -lhs
            } else {
                lhs
            }
        }
    };
    canonicalize_number(value)
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_number, floored_mod};

    fn reference_floored_mod(lhs: f64, rhs: f64) -> f64 {
        let mut value = libm::fmod(lhs, rhs);
        if value != 0.0 && ((value < 0.0) != (rhs < 0.0)) {
            value += rhs;
        }
        canonicalize_number(value)
    }

    #[test]
    fn positive_integer_mod_matches_reference() {
        for rhs in [1_u64, 2, 3, 7, 8, 10, 31, 256, 1_000, 65_535] {
            for lhs in 0..=10_000_u64 {
                assert_eq!(
                    floored_mod(lhs as f64, rhs as f64).to_bits(),
                    reference_floored_mod(lhs as f64, rhs as f64).to_bits(),
                    "lhs={lhs} rhs={rhs}",
                );
            }
        }

        for (lhs, rhs) in [
            (9_007_199_254_740_991.0, 8.0),
            (9_007_199_254_740_992.0, 8.0),
            (18_446_744_073_709_549_568.0, 4_096.0),
        ] {
            assert_eq!(
                floored_mod(lhs, rhs).to_bits(),
                reference_floored_mod(lhs, rhs).to_bits(),
            );
        }
    }

    #[test]
    fn non_fast_path_mod_cases_preserve_reference_bits() {
        for (lhs, rhs) in [
            (-0.0, 8.0),
            (-9.0, 8.0),
            (9.5, 8.0),
            (9.0, -8.0),
            (f64::INFINITY, 8.0),
            (9.0, f64::INFINITY),
            (f64::NAN, 8.0),
        ] {
            assert_eq!(
                floored_mod(lhs, rhs).to_bits(),
                reference_floored_mod(lhs, rhs).to_bits(),
                "lhs={lhs:?} rhs={rhs:?}",
            );
        }
    }
}
