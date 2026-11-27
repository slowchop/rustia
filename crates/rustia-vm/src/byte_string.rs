//! Shared byte-string operations used by the VM and compiled embedders.
//!
//! The caller owns value coercion, result storage and resource accounting.

use crate::{Error, ErrorKind, Result};

pub fn string_library_index(value: f64) -> Result<i64> {
    // The half-open range also rejects NaN and infinities. At either boundary,
    // f64 spacing exceeds one, so checking before truncation is equivalent.
    // The checked cast below supplies truncation without a separate operation.
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer string library index",
        }));
    }
    Ok(value as i64)
}

pub fn luau_string_sub(text: &[u8], start: i64, end: i64) -> Result<Vec<u8>> {
    let len = i64::try_from(text.len()).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let start = if start < 0 { len + start + 1 } else { start }.max(1);
    let end = if end < 0 { len + end + 1 } else { end }.min(len);
    if start > end {
        return Ok(Vec::new());
    }
    let start = usize::try_from(start - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.sub start index underflow",
        })
    })?;
    let end = usize::try_from(end).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.sub end index underflow",
        })
    })?;
    let bytes = text.get(start..end).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.sub byte range is out of bounds",
        })
    })?;
    Ok(bytes.to_vec())
}

fn normalize_string_byte_start(
    len: usize,
    index: i64,
    clamp_to_first: bool,
) -> Result<Option<usize>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let mut one_based = if index < 0 { len + index + 1 } else { index };
    if one_based < 1 && clamp_to_first && len > 0 {
        one_based = 1;
    }
    if one_based < 1 || one_based > len {
        return Ok(None);
    }
    Ok(Some(usize::try_from(one_based - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string index underflow",
        })
    })?))
}

fn normalize_string_byte_end(len: usize, index: i64) -> Result<Option<usize>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let one_based = if index < 0 { len + index + 1 } else { index };
    if one_based < 1 {
        return Ok(None);
    }
    let one_based = one_based.min(len);
    Ok(Some(usize::try_from(one_based - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string index underflow",
        })
    })?))
}

pub fn reverse_string_bytes(text: &[u8]) -> Result<Vec<u8>> {
    Ok(text.iter().rev().copied().collect::<Vec<_>>())
}

/// Select bytes using Luau's one-based and negative indices.
/// The explicit-end form clamps a nonpositive start to the first byte.
pub fn string_byte_slice(text: &[u8], start: i64, end: i64, explicit_end: bool) -> Result<&[u8]> {
    let Some(start) = normalize_string_byte_start(text.len(), start, explicit_end)? else {
        return Ok(&[]);
    };
    let Some(end) = normalize_string_byte_end(text.len(), end)? else {
        return Ok(&[]);
    };
    Ok(text.get(start..=end).unwrap_or_default())
}

/// Convert one already-coerced string.char argument to a byte.
pub fn char_byte(number: f64) -> Result<u8> {
    let truncated = number.trunc();
    if !truncated.is_finite() || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&truncated) {
        return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
    }
    u8::try_from(truncated as i32).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "byte value",
        })
    })
}

const MAX_STRING_REPEAT_BYTES: usize = 1 << 30;

pub fn string_repeat_count(value: f64) -> Result<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer string.rep count",
        }));
    }
    Ok(value as i64)
}

pub(crate) fn string_repeat_len(text_len: usize, count: i64) -> Result<usize> {
    if count <= 0 || text_len == 0 {
        return Ok(0);
    }
    let count = usize::try_from(count).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "non-negative string.rep count",
        })
    })?;
    let total_len = text_len.checked_mul(count).ok_or_else(|| {
        Error::new(ErrorKind::TypeError {
            expected: "string.rep result below allocation limit",
        })
    })?;
    if total_len > MAX_STRING_REPEAT_BYTES {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "string.rep result below allocation limit",
        }));
    }
    Ok(total_len)
}

pub fn repeat_string(text: &[u8], count: i64) -> Result<Vec<u8>> {
    let total_len = string_repeat_len(text.len(), count)?;
    if total_len == 0 {
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "non-negative string.rep count",
        })
    })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(total_len)
        .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
    for _ in 0..count {
        output.extend_from_slice(text);
    }
    Ok(output)
}

pub fn split_luau_string(text: &[u8], separator: &[u8]) -> Vec<Vec<u8>> {
    split_parts(text, separator).map(<[u8]>::to_vec).collect()
}

/// Borrow each piece, including trailing empty pieces for nonempty separators.
pub(crate) fn split_parts<'a>(
    text: &'a [u8],
    separator: &'a [u8],
) -> impl Iterator<Item = &'a [u8]> {
    let mut remaining = Some(text);
    std::iter::from_fn(move || {
        let text = remaining.take()?;
        if separator.is_empty() {
            let (part, tail) = text.split_at_checked(1)?;
            remaining = Some(tail);
            return Some(part);
        }
        if let Some(end) = text
            .windows(separator.len())
            .position(|part| part == separator)
        {
            remaining = Some(&text[end + separator.len()..]);
            Some(&text[..end])
        } else {
            Some(text)
        }
    })
}
