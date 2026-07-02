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
        match self.0.tag {
            TAG_NIL => ValueKind::Nil,
            TAG_FALSE | TAG_TRUE => ValueKind::Boolean,
            TAG_NUMBER => ValueKind::Number,
            TAG_STRING_ATOM => ValueKind::String,
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
                self.as_string_atom() == other.as_string_atom()
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
