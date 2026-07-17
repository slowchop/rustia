use crate::number::{canonicalize_number, table_key_number_bits};
use gc_arena::Collect;
use std::fmt;

const TAG_NIL: u64 = 0;
const TAG_FALSE: u64 = 1;
const TAG_TRUE: u64 = 2;
const TAG_NUMBER: u64 = 3;
const TAG_STRING_ATOM: u64 = 4;
const TAG_TABLE: u64 = 5;
const TAG_FUNCTION: u64 = 6;
const TAG_USERDATA: u64 = 7;
const TAG_BUFFER: u64 = 8;
const TAG_VECTOR: u64 = 9;
const TAG_SHORT_STRING: u64 = 10;
const TAG_MASK: u64 = 0xff;
const SHORT_STRING_LENGTH_SHIFT: u32 = 8;
const SHORT_STRING_BYTES_SHIFT: u32 = 16;
// Keep the top payload byte reserved so the packed form has a canonical zero byte available for
// future value-representation evolution.
pub(crate) const SHORT_STRING_CAPACITY: usize = 13;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Collect)]
#[collect(no_drop)]
pub(crate) struct ShortString {
    bytes: [u8; SHORT_STRING_CAPACITY],
    len: u8,
}

impl ShortString {
    #[inline]
    pub(crate) fn new(value: &[u8]) -> Option<Self> {
        if value.len() > SHORT_STRING_CAPACITY {
            return None;
        }
        let mut bytes = [0_u8; SHORT_STRING_CAPACITY];
        bytes[..value.len()].copy_from_slice(value);
        Some(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    #[inline]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq, Collect)]
#[collect(no_drop)]
struct ValueRepr {
    tag: u64,
    payload: u64,
}

/// A script value.
///
/// The representation is deliberately sealed. P1 starts with the locked 16-byte tagged layout,
/// but `design/CURRENT.md` keeps the JSC/Firefox wasm measurement as a pre-lock gate, so callers
/// must not be able to depend on the payload shape.
#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub struct Value(ValueRepr);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValueKind {
    Nil,
    Boolean,
    Number,
    String,
    Table,
    Function,
    Userdata,
    Buffer,
    Vector,
}

impl Value {
    #[inline]
    pub const fn nil() -> Self {
        Self(ValueRepr {
            tag: TAG_NIL,
            payload: 0,
        })
    }

    #[inline]
    pub const fn boolean(value: bool) -> Self {
        if value {
            Self(ValueRepr {
                tag: TAG_TRUE,
                payload: 1,
            })
        } else {
            Self(ValueRepr {
                tag: TAG_FALSE,
                payload: 0,
            })
        }
    }

    #[inline]
    pub fn number(value: f64) -> Self {
        Self(ValueRepr {
            tag: TAG_NUMBER,
            payload: canonicalize_number(value).to_bits(),
        })
    }

    #[inline]
    pub(crate) const fn string_atom(atom: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_STRING_ATOM,
            payload: atom as u64,
        })
    }

    /// Stores a short binary string directly in the sealed 16-byte value representation.
    ///
    /// The low tag byte remains the discriminator, the next byte stores the length, and the
    /// remaining fourteen bytes are split across the tag and payload words; thirteen carry
    /// string bytes and the final payload byte stays zero. Explicit shifts keep the representation
    /// independent of host endianness.
    #[inline]
    pub(crate) fn short_string(value: &[u8]) -> Option<Self> {
        let short = ShortString::new(value)?;
        let value = short.as_bytes();
        let mut tag = TAG_SHORT_STRING | ((value.len() as u64) << SHORT_STRING_LENGTH_SHIFT);
        let mut payload = 0_u64;
        for (index, byte) in value.iter().copied().enumerate() {
            if index < 6 {
                tag |= u64::from(byte) << (SHORT_STRING_BYTES_SHIFT + index as u32 * 8);
            } else {
                payload |= u64::from(byte) << ((index as u32 - 6) * 8);
            }
        }
        Some(Self(ValueRepr { tag, payload }))
    }

    #[inline]
    pub(crate) const fn table(handle: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_TABLE,
            payload: handle as u64,
        })
    }

    #[inline]
    pub(crate) const fn function(handle: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_FUNCTION,
            payload: handle as u64,
        })
    }

    #[inline]
    pub(crate) const fn userdata(handle: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_USERDATA,
            payload: handle as u64,
        })
    }

    #[inline]
    pub(crate) const fn buffer(handle: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_BUFFER,
            payload: handle as u64,
        })
    }

    #[inline]
    pub(crate) const fn vector(handle: u32) -> Self {
        Self(ValueRepr {
            tag: TAG_VECTOR,
            payload: handle as u64,
        })
    }

    #[inline]
    pub const fn kind(self) -> ValueKind {
        match self.0.tag & TAG_MASK {
            TAG_NIL => ValueKind::Nil,
            TAG_FALSE | TAG_TRUE => ValueKind::Boolean,
            TAG_NUMBER => ValueKind::Number,
            TAG_STRING_ATOM | TAG_SHORT_STRING => ValueKind::String,
            TAG_TABLE => ValueKind::Table,
            TAG_FUNCTION => ValueKind::Function,
            TAG_USERDATA => ValueKind::Userdata,
            TAG_BUFFER => ValueKind::Buffer,
            TAG_VECTOR => ValueKind::Vector,
            _ => ValueKind::Nil,
        }
    }

    #[inline]
    pub const fn as_bool(self) -> Option<bool> {
        match self.0.tag {
            TAG_FALSE => Some(false),
            TAG_TRUE => Some(true),
            _ => None,
        }
    }

    #[inline]
    pub fn as_number(self) -> Option<f64> {
        (self.0.tag == TAG_NUMBER).then(|| f64::from_bits(self.0.payload))
    }

    #[inline]
    pub(crate) const fn as_string_atom(self) -> Option<u32> {
        if self.0.tag == TAG_STRING_ATOM {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn as_short_string(self) -> Option<ShortString> {
        if self.0.tag & TAG_MASK != TAG_SHORT_STRING {
            return None;
        }
        let len = ((self.0.tag >> SHORT_STRING_LENGTH_SHIFT) & 0xff) as u8;
        if usize::from(len) > SHORT_STRING_CAPACITY {
            return None;
        }
        let mut bytes = [0_u8; SHORT_STRING_CAPACITY];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = if index < 6 {
                (self.0.tag >> (SHORT_STRING_BYTES_SHIFT + index as u32 * 8)) as u8
            } else {
                (self.0.payload >> ((index as u32 - 6) * 8)) as u8
            };
        }
        Some(ShortString { bytes, len })
    }

    #[inline]
    pub(crate) const fn as_table(self) -> Option<u32> {
        if self.0.tag == TAG_TABLE {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn as_function(self) -> Option<u32> {
        if self.0.tag == TAG_FUNCTION {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn as_userdata(self) -> Option<u32> {
        if self.0.tag == TAG_USERDATA {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn as_buffer(self) -> Option<u32> {
        if self.0.tag == TAG_BUFFER {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn as_vector(self) -> Option<u32> {
        if self.0.tag == TAG_VECTOR {
            Some(self.0.payload as u32)
        } else {
            None
        }
    }

    #[inline]
    pub const fn truthy(self) -> bool {
        !matches!(self.0.tag, TAG_NIL | TAG_FALSE)
    }

    #[inline]
    pub(crate) fn table_key_number_bits(self) -> Option<u64> {
        self.as_number().map(table_key_number_bits)
    }

    pub(crate) fn luau_equals(self, other: Self) -> bool {
        match (self.kind(), other.kind()) {
            (ValueKind::Nil, ValueKind::Nil) => true,
            (ValueKind::Boolean, ValueKind::Boolean) => self.as_bool() == other.as_bool(),
            (ValueKind::Number, ValueKind::Number) => self.as_number() == other.as_number(),
            (ValueKind::String, ValueKind::String) => {
                match (self.as_short_string(), other.as_short_string()) {
                    (Some(lhs), Some(rhs)) => lhs == rhs,
                    (None, None) => self.as_string_atom() == other.as_string_atom(),
                    _ => false,
                }
            }
            (ValueKind::Table, ValueKind::Table) => self.as_table() == other.as_table(),
            (ValueKind::Function, ValueKind::Function) => self.as_function() == other.as_function(),
            (ValueKind::Userdata, ValueKind::Userdata) => self.as_userdata() == other.as_userdata(),
            (ValueKind::Buffer, ValueKind::Buffer) => self.as_buffer() == other.as_buffer(),
            (ValueKind::Vector, ValueKind::Vector) => false,
            _ => false,
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::nil()
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            ValueKind::Nil => formatter.write_str("nil"),
            ValueKind::Boolean => formatter
                .debug_tuple("boolean")
                .field(&self.as_bool().unwrap_or(false))
                .finish(),
            ValueKind::Number => formatter
                .debug_tuple("number")
                .field(&self.as_number().unwrap_or(f64::NAN))
                .finish(),
            ValueKind::String => formatter.write_str("string"),
            ValueKind::Table => formatter.write_str("table"),
            ValueKind::Function => formatter.write_str("function"),
            ValueKind::Userdata => formatter.write_str("userdata"),
            ValueKind::Buffer => formatter.write_str("buffer"),
            ValueKind::Vector => formatter.write_str("vector"),
        }
    }
}

const _: () = {
    assert!(std::mem::size_of::<Value>() == 16);
    assert!(std::mem::align_of::<Value>() == 8);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_strings_round_trip_binary_bytes_in_the_locked_value_layout() {
        for len in 0..=SHORT_STRING_CAPACITY {
            let bytes = (0..len)
                .map(|index| (index as u8).wrapping_mul(37).wrapping_add(0x80))
                .collect::<Vec<_>>();
            let value = Value::short_string(&bytes).expect("short string should fit");
            assert_eq!(value.kind(), ValueKind::String);
            assert_eq!(
                value
                    .as_short_string()
                    .map(|value| value.as_bytes().to_vec()),
                Some(bytes)
            );
            assert_eq!(
                value.0.payload >> 56,
                0,
                "reserved payload byte must be zero"
            );
        }
        assert!(Value::short_string(&[0xff; SHORT_STRING_CAPACITY + 1]).is_none());
    }

    #[test]
    fn short_string_equality_uses_canonical_packed_bytes() {
        let lhs = Value::short_string(b"field\0name").expect("short string");
        let rhs = Value::short_string(b"field\0name").expect("short string");
        let other = Value::short_string(b"field\0names").expect("short string");
        assert!(lhs.luau_equals(rhs));
        assert!(!lhs.luau_equals(other));
        assert!(!lhs.luau_equals(Value::string_atom(0)));
    }
}
