//! Deterministic f64 operations shared by the VM and compiler constant folder.

/// The single quiet-NaN payload rustia uses for every script-number NaN.
pub const CANON_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0;

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
    if let Some(value) = positive_integer_mod(lhs, rhs) {
        return value;
    }
    let mut value = libm::fmod(lhs, rhs);
    if value != 0.0 && ((value < 0.0) != (rhs < 0.0)) {
        value += rhs;
    }
    canonicalize_number(value)
}

#[inline]
fn positive_integer_mod(lhs: f64, rhs: f64) -> Option<f64> {
    if lhs == 0.0 && rhs > 0.0 {
        return Some(lhs);
    }
    if lhs >= 0.0
        && rhs > 0.0
        && lhs <= MAX_EXACT_F64_INTEGER
        && rhs <= MAX_EXACT_F64_INTEGER
        && lhs.fract() == 0.0
        && rhs.fract() == 0.0
    {
        let rhs = rhs as u64;
        if rhs != 0 {
            return Some(((lhs as u64) % rhs) as f64);
        }
    }
    None
}

#[inline]
pub fn floor_div(lhs: f64, rhs: f64) -> f64 {
    canonicalize_number(libm::floor(lhs / rhs))
}

#[inline(always)]
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
    use super::floored_mod;

    #[test]
    fn floored_mod_fast_path_preserves_luau_edges() {
        assert_eq!(floored_mod(17.0, 8.0), 1.0);
        assert_eq!(floored_mod(17.5, 8.0), 1.5);
        assert_eq!(floored_mod(-3.0, 2.0), 1.0);
        assert_eq!(floored_mod(3.0, -2.0), -1.0);
        assert_eq!(floored_mod(-0.0, 2.0).to_bits(), (-0.0f64).to_bits());
    }
}
