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

#[cfg(any(target_arch = "x86_64", test))]
#[inline]
fn is_integral_bits(value: f64) -> bool {
    let bits = value.to_bits();
    let biased_exponent = ((bits >> 52) & 0x7ff) as i32;
    let exponent = biased_exponent - 1023;
    if exponent < 0 {
        value == 0.0
    } else if exponent >= 52 {
        biased_exponent != 0x7ff
    } else {
        let fractional_mask = (1_u64 << (52 - exponent)) - 1;
        bits & fractional_mask == 0
    }
}

#[inline]
fn is_integral_mod_operand(value: f64) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // Rust's x86-64 `f64::fract` currently makes an out-of-line truncation call.
        is_integral_bits(value)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        value.fract() == 0.0
    }
}

#[inline]
pub fn floored_mod(lhs: f64, rhs: f64) -> f64 {
    // Integer counters and table/hash arithmetic commonly use a positive integer
    // modulus. Every integer represented by `lhs` in this range converts exactly to
    // `u64`, and every possible remainder converts exactly back to `f64`, so integer
    // remainder has the same result as `fmod` without entering libm.
    if !lhs.is_sign_negative()
        && (0.0..18_446_744_073_709_551_616.0).contains(&lhs)
        && is_integral_mod_operand(lhs)
        && (1.0..=9_007_199_254_740_992.0).contains(&rhs)
        && is_integral_mod_operand(rhs)
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
    use super::{canonicalize_number, floored_mod, is_integral_bits};

    #[test]
    fn ieee_integral_predicate_matches_numeric_integrality() {
        for (value, expected) in [
            (0.0, true),
            (-0.0, true),
            (1.0, true),
            (-1.0, true),
            (1.5, false),
            (-1.5, false),
            (f64::from_bits(1), false),
            (f64::MIN_POSITIVE, false),
            (4_503_599_627_370_495.0, true),
            (4_503_599_627_370_496.0, true),
            (f64::MAX, true),
            (f64::INFINITY, false),
            (f64::NEG_INFINITY, false),
            (f64::NAN, false),
            (f64::from_bits(0xfff8_0000_0000_0042), false),
        ] {
            assert_eq!(is_integral_bits(value), expected, "value={value:?}");
        }
    }

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
