//! Stateless numeric kernels for the standard math library.
//!
//! Embedders validate arity and coerce values before calling these operations.
//! Stateful operations such as random number generation belong to the VM.

use super::canonicalize_number;

const PI: f64 = std::f64::consts::PI;

#[derive(Clone, Copy)]
pub enum MathUnary {
    Floor,
    Ceil,
    Round,
    Sqrt,
    Deg,
    Rad,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Log10,
    Sign,
}

impl MathUnary {
    #[inline]
    pub fn apply(self, value: f64) -> f64 {
        let result = match self {
            Self::Floor => libm::floor(value),
            Self::Ceil => libm::ceil(value),
            Self::Round => libm::round(value),
            Self::Sqrt => libm::sqrt(value),
            Self::Deg => value * (180.0 / PI),
            Self::Rad => value * (PI / 180.0),
            Self::Sin => libm::sin(value),
            Self::Cos => libm::cos(value),
            Self::Tan => libm::tan(value),
            Self::Asin => libm::asin(value),
            Self::Acos => libm::acos(value),
            Self::Atan => libm::atan(value),
            Self::Sinh => libm::sinh(value),
            Self::Cosh => libm::cosh(value),
            Self::Tanh => libm::tanh(value),
            Self::Exp => libm::exp(value),
            Self::Log10 => libm::log10(value),
            Self::Sign => {
                if value > 0.0 {
                    1.0
                } else if value < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
        };
        canonicalize_number(result)
    }
}

#[derive(Clone, Copy)]
pub enum MathBinary {
    Fmod,
    Pow,
    Atan2,
}

impl MathBinary {
    #[inline]
    pub fn apply(self, lhs: f64, rhs: f64) -> f64 {
        let result = match self {
            Self::Fmod => libm::fmod(lhs, rhs),
            Self::Pow => libm::pow(lhs, rhs),
            Self::Atan2 => libm::atan2(lhs, rhs),
        };
        canonicalize_number(result)
    }
}

#[derive(Clone, Copy)]
pub enum MathMinMax {
    Min,
    Max,
}

impl MathMinMax {
    /// Fold one argument in input order, keeping the earlier value on ties.
    #[inline]
    pub fn step(self, current: f64, value: f64) -> f64 {
        match self {
            Self::Min if value < current => value,
            Self::Max if value > current => value,
            _ => current,
        }
    }
}

pub fn log(value: f64, base: Option<f64>) -> f64 {
    canonicalize_number(match base {
        None => libm::log(value),
        Some(2.0) => libm::log2(value),
        Some(10.0) => libm::log10(value),
        Some(base) => libm::log(value) / libm::log(base),
    })
}

/// Scale the mantissa by a power of two with a signed 32-bit integer exponent.
pub fn ldexp(mantissa: f64, exponent: f64) -> Option<f64> {
    if exponent.fract() != 0.0 || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&exponent) {
        return None;
    }
    Some(canonicalize_number(libm::ldexp(mantissa, exponent as i32)))
}

pub fn clamp(value: f64, min: f64, max: f64) -> Option<f64> {
    if min > max || min.is_nan() || max.is_nan() {
        return None;
    }
    let result = if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    };
    Some(canonicalize_number(result))
}

pub fn map(value: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
    canonicalize_number(out_min + (value - in_min) * (out_max - out_min) / (in_max - in_min))
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    canonicalize_number(if t == 1.0 { b } else { a + (b - a) * t })
}

/// Return the mantissa followed by the exponent.
pub fn frexp(value: f64) -> (f64, f64) {
    let (mantissa, exponent) = libm::frexp(value);
    (canonicalize_number(mantissa), f64::from(exponent))
}

/// Return the whole part followed by the fraction.
pub fn modf(value: f64) -> (f64, f64) {
    let (fraction, integer) = libm::modf(value);
    (canonicalize_number(integer), canonicalize_number(fraction))
}
