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
