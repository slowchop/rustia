//! Deterministic numeric operations shared by the interpreter and compiled code.

pub mod math;

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

/// Convert a Luau byte string using the VM's default numeric coercion.
pub fn parse_numeric_text(text: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(text).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    if parse_nan_string(text) {
        return Some(canonicalize_number(f64::NAN));
    }
    if let Some(number) = parse_hex_number_string(text) {
        return Some(canonicalize_number(number));
    }
    text.parse::<f64>().ok().map(canonicalize_number)
}

fn parse_nan_string(text: &str) -> bool {
    let payload = match text.as_bytes() {
        [b'+', rest @ ..] | [b'-', rest @ ..] => rest,
        _ => text.as_bytes(),
    };

    if payload.eq_ignore_ascii_case(b"nan") {
        return true;
    }

    if payload.len() < 5 || !payload[..3].eq_ignore_ascii_case(b"nan") || payload[3] != b'(' {
        return false;
    }

    let Some(close) = payload[4..]
        .iter()
        .position(|byte| *byte == b')')
        .map(|offset| offset + 4)
    else {
        return false;
    };
    close + 1 == payload.len()
}

/// Convert numeric text using the explicit-base `tonumber` rule.
pub fn parse_numeric_text_in_base(text: &[u8], base: u32) -> Option<f64> {
    if base == 10 {
        return parse_numeric_text(text);
    }
    if !(2..=36).contains(&base) {
        return None;
    }
    let text = std::str::from_utf8(text).ok()?.as_bytes();
    let mut cursor = 0;
    while text
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }
    if cursor == text.len() {
        return None;
    }

    let negative = match text[cursor] {
        b'+' => {
            cursor += 1;
            false
        }
        b'-' => {
            cursor += 1;
            true
        }
        _ => false,
    };

    if base == 16
        && text.get(cursor) == Some(&b'0')
        && text
            .get(cursor + 1)
            .is_some_and(|byte| matches!(byte, b'x' | b'X'))
        && text
            .get(cursor + 2)
            .and_then(|byte| tonumber_digit(*byte))
            .is_some_and(|digit| digit < base)
    {
        cursor += 2;
    }

    let mut value = 0_u64;
    let mut overflowed = false;
    let mut parsed_digit = false;
    while let Some(byte) = text.get(cursor).copied() {
        let Some(digit) = tonumber_digit(byte) else {
            break;
        };
        if digit >= base {
            break;
        }
        parsed_digit = true;
        cursor += 1;
        if !overflowed {
            match value
                .checked_mul(u64::from(base))
                .and_then(|value| value.checked_add(u64::from(digit)))
            {
                Some(next) => value = next,
                None => {
                    value = u64::MAX;
                    overflowed = true;
                }
            }
        }
    }

    if !parsed_digit
        || text[cursor..]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }

    if negative && !overflowed {
        value = 0_u64.wrapping_sub(value);
    }
    Some(canonicalize_number(value as f64))
}

fn tonumber_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn parse_hex_number_string(text: &str) -> Option<f64> {
    let (sign, digits) = match text.as_bytes() {
        [b'+', rest @ ..] => (1.0, std::str::from_utf8(rest).ok()?),
        [b'-', rest @ ..] => (-1.0, std::str::from_utf8(rest).ok()?),
        _ => (1.0, text),
    };
    let digits = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))?;

    let mut cursor = 0;
    let bytes = digits.as_bytes();
    let mut value = 0.0;
    let mut parsed_digit = false;
    while let Some(digit) = bytes
        .get(cursor)
        .and_then(|byte| tonumber_digit(*byte))
        .filter(|digit| *digit < 16)
    {
        value = value * 16.0 + f64::from(digit);
        parsed_digit = true;
        cursor += 1;
    }

    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let mut place = 1.0 / 16.0;
        while let Some(digit) = bytes
            .get(cursor)
            .and_then(|byte| tonumber_digit(*byte))
            .filter(|digit| *digit < 16)
        {
            value += f64::from(digit) * place;
            place /= 16.0;
            parsed_digit = true;
            cursor += 1;
        }
    }

    if !parsed_digit {
        return None;
    }

    if bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b'p' | b'P'))
    {
        cursor += 1;
        let exponent_sign = match bytes.get(cursor) {
            Some(b'+') => {
                cursor += 1;
                1
            }
            Some(b'-') => {
                cursor += 1;
                -1
            }
            _ => 1,
        };
        let mut exponent = 0_i32;
        let mut parsed_exponent_digit = false;
        while let Some(digit) = bytes
            .get(cursor)
            .copied()
            .filter(u8::is_ascii_digit)
            .map(|byte| i32::from(byte - b'0'))
        {
            exponent = exponent.saturating_mul(10).saturating_add(digit);
            parsed_exponent_digit = true;
            cursor += 1;
        }
        if !parsed_exponent_digit {
            return None;
        }
        value = libm::ldexp(value, exponent.saturating_mul(exponent_sign));
    }

    if cursor != bytes.len() {
        return None;
    }

    Some(sign * value)
}

/// Format a number using the normal Luau VM text representation.
pub fn luau_number_to_string(value: f64) -> String {
    let value = canonicalize_number(value);
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value == f64::INFINITY {
        return "inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }
    if value == 0.0 {
        if value.is_sign_negative() {
            return "-0".to_owned();
        }
        return "0".to_owned();
    }
    const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
    if value.fract() == 0.0 && value.abs() <= MAX_EXACT_INTEGER {
        return (value as i64).to_string();
    }
    let debug = format!("{value:?}");
    let (negative, magnitude) = debug
        .strip_prefix('-')
        .map_or((false, debug.as_str()), |magnitude| (true, magnitude));
    let mut output =
        luau_number_magnitude_to_string(magnitude).unwrap_or_else(|| value.to_string());
    if negative {
        output.insert(0, '-');
    }
    output
}

fn luau_number_magnitude_to_string(magnitude: &str) -> Option<String> {
    let (mantissa, exponent) = magnitude
        .split_once('e')
        .or_else(|| magnitude.split_once('E'))
        .map_or(Some((magnitude, 0)), |(mantissa, exponent)| {
            exponent
                .parse::<i32>()
                .ok()
                .map(|exponent| (mantissa, exponent))
        })?;
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() || !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut raw_digits = String::with_capacity(integer.len() + fraction.len());
    raw_digits.push_str(integer);
    raw_digits.push_str(fraction);
    let leading_zeroes = raw_digits
        .bytes()
        .position(|byte| byte != b'0')
        .unwrap_or(raw_digits.len());
    let mut digits = raw_digits[leading_zeroes..].to_owned();
    while digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        return Some("0".to_owned());
    }
    let decimal_exponent = exponent + integer.len() as i32 - leading_zeroes as i32 - 1;
    Some(luau_decimal_digits_to_string(&digits, decimal_exponent))
}

fn luau_decimal_digits_to_string(digits: &str, decimal_exponent: i32) -> String {
    if !(-6..21).contains(&decimal_exponent) {
        return luau_decimal_digits_to_exponent_string(digits, decimal_exponent);
    }

    let integer_digits = decimal_exponent + 1;
    if integer_digits <= 0 {
        let mut output = String::from("0.");
        output.extend(std::iter::repeat_n('0', (-integer_digits) as usize));
        output.push_str(digits);
        return output;
    }

    let integer_digits = integer_digits as usize;
    if integer_digits >= digits.len() {
        let mut output = String::from(digits);
        output.extend(std::iter::repeat_n('0', integer_digits - digits.len()));
        return output;
    }

    let mut output = String::with_capacity(digits.len() + 1);
    output.push_str(&digits[..integer_digits]);
    output.push('.');
    output.push_str(&digits[integer_digits..]);
    output
}

fn luau_decimal_digits_to_exponent_string(digits: &str, decimal_exponent: i32) -> String {
    let mut output = String::with_capacity(digits.len() + 8);
    output.push_str(&digits[..1]);
    if digits.len() > 1 {
        output.push('.');
        output.push_str(&digits[1..]);
    }
    output.push('e');
    if decimal_exponent >= 0 {
        output.push('+');
    }
    output.push_str(&decimal_exponent.to_string());
    output
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_number, floored_mod, is_integral_bits, parse_numeric_text, CANON_NAN_BITS,
    };

    #[test]
    fn numeric_text_matches_vm_coercion() {
        assert_eq!(parse_numeric_text(b" 0x1.8p+2 "), Some(6.0));
        assert_eq!(parse_numeric_text(b" -2.5e1 "), Some(-25.0));
        assert_eq!(
            parse_numeric_text(b"-0x0").unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(
            parse_numeric_text(b"NaN(payload)").unwrap().to_bits(),
            CANON_NAN_BITS
        );
        assert_eq!(parse_numeric_text(b"1\xff"), None);
        assert_eq!(parse_numeric_text(b"0x.p1"), None);
    }

    #[test]
    fn number_conversion_helpers_keep_the_luau_cases() {
        use super::{luau_number_to_string, parse_numeric_text_in_base};
        assert_eq!(parse_numeric_text_in_base(b" ff ", 16), Some(255.0));
        assert_eq!(parse_numeric_text_in_base(b" 0x1.8p+2 ", 10), Some(6.0));
        assert_eq!(parse_numeric_text_in_base(b"101", 2), Some(5.0));
        assert_eq!(parse_numeric_text_in_base(b"101", 1), None);
        assert_eq!(luau_number_to_string(-0.0), "-0");
        assert_eq!(luau_number_to_string(f64::INFINITY), "inf");
        assert_eq!(luau_number_to_string(1.0e21), "1e+21");
    }

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
