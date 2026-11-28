//! Portable table index and range rules shared by the VM and compiled callers.
//! Callers coerce values, enforce their resource policy and own table storage.

use crate::error::{Error, ErrorKind, Result};

const LUAU_TABLE_MAX_SIZE: usize = 1 << 26;
const MAX_TABLE_CREATE_ARRAY_SIZE: usize = 100_000;
pub(crate) const MAX_LUAU_TABLE_INDEX: f64 = i32::MAX as f64;

/// Validate the size and apply the standard builtin's allocation limit.
pub fn table_create_size(value: f64) -> Result<usize> {
    table_create_size_with_limit(value, true)
}

/// Check an explicit index for `table.concat`.
pub fn table_concat_index(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=MAX_LUAU_TABLE_INDEX).contains(&value)
    {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.concat range",
        }));
    }
    Ok(value as usize)
}

/// Check a non-nil initial index for `table.find`.
pub fn table_find_start(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.find init",
        }));
    }
    if !(1.0..=MAX_LUAU_TABLE_INDEX).contains(&value) {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "index out of range",
        }));
    }
    Ok(value as usize)
}

pub(crate) fn table_create_size_with_limit(value: f64, apply_create_cap: bool) -> Result<usize> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.create size",
        }));
    }
    let size = value as i32;
    if size < 0 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "size out of range",
        }));
    }
    let size = size as usize;
    if size > LUAU_TABLE_MAX_SIZE {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "table overflow",
        }));
    }
    if apply_create_cap && size > MAX_TABLE_CREATE_ARRAY_SIZE {
        return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
    }
    Ok(size)
}

/// Truncate a coerced table position after checking the signed 32-bit range.
pub fn table_integer_arg(value: f64, expected: &'static str) -> Result<i32> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError { expected }));
    }
    Ok(value as i32)
}

/// Check an explicit `table.insert` position. Luau maps NaN to index zero.
pub fn table_insert_position_arg(value: f64) -> Result<i32> {
    if value.is_nan() {
        return Ok(0);
    }
    table_integer_arg(value, "integer table.insert position")
}

pub(crate) fn table_move_count(start: i32, end: i32) -> Result<usize> {
    if end < start {
        return Ok(0);
    }
    let count = i64::from(end) - i64::from(start) + 1;
    if count > i64::from(i32::MAX) {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        }));
    }
    usize::try_from(count).map_err(|_| {
        Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        })
    })
}

fn table_move_destination_end(destination_start: i32, count: usize) -> Result<i32> {
    let count = i64::try_from(count).map_err(|_| {
        Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        })
    })?;
    if count == 0 {
        return Ok(destination_start);
    }
    let destination_end = i64::from(destination_start) + count - 1;
    if destination_end < i64::from(i32::MIN) || destination_end > i64::from(i32::MAX) {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "destination wrap around".to_owned(),
        }));
    }
    Ok(destination_end as i32)
}

/// The ordered source and destination indices of a checked `table.move`.
/// An overlapping move within one table reads from the end when needed to
/// avoid overwriting unread values. An empty range has no indices.
pub fn table_move_indices(
    start: i32,
    end: i32,
    destination_start: i32,
    same_table: bool,
) -> Result<impl Iterator<Item = (i32, i32)>> {
    let count = table_move_count(start, end)?;
    table_move_destination_end(destination_start, count)?;
    let count = count as i32;
    let backward = same_table && destination_start > start && destination_start <= end;
    Ok((0..count).map(move |offset| {
        let offset = if backward { count - offset - 1 } else { offset };
        (start + offset, destination_start + offset)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_moves_read_values_in_the_safe_order() {
        let indices = |start, destination, same_table| {
            table_move_indices(start, start + 2, destination, same_table)
                .unwrap()
                .collect::<Vec<_>>()
        };
        assert_eq!(indices(1, 2, true), [(3, 4), (2, 3), (1, 2)]);
        assert_eq!(indices(2, 1, true), [(2, 1), (3, 2), (4, 3)]);
        assert_eq!(indices(1, 2, false), [(1, 2), (2, 3), (3, 4)]);
    }

    #[test]
    fn move_bounds_are_checked_before_producing_indices() {
        assert!(table_move_indices(2, 1, i32::MAX, true)
            .unwrap()
            .next()
            .is_none());
        assert!(table_move_indices(i32::MIN, i32::MAX, 1, false).is_err());
        assert!(table_move_indices(1, 2, i32::MAX, false).is_err());
        assert_eq!(
            table_move_indices(i32::MAX, i32::MAX, i32::MIN, false)
                .unwrap()
                .next(),
            Some((i32::MAX, i32::MIN))
        );
    }
}
