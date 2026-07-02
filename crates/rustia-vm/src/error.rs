use crate::profile::Profile;
use crate::value::Value;
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub pc: Option<usize>,
    pub location: Option<ErrorLocation>,
    pub location_suppressed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorLocation {
    pub source: String,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum ErrorKind {
    FuelExhausted {
        limit: u64,
    },
    RecursionLimit {
        limit: usize,
    },
    RegisterOutOfBounds {
        register: usize,
        max: usize,
    },
    ConstantOutOfBounds {
        index: usize,
        len: usize,
    },
    TypeError {
        expected: &'static str,
    },
    OutOfMemory,
    TableIndexNil,
    TableIndexNaN,
    InvalidKeyToNext,
    AssertionFailed {
        message: String,
    },
    RuntimeError {
        message: String,
    },
    GuestError {
        value: Value,
        message: String,
    },
    Unsupported {
        feature: &'static str,
    },
    CapabilityRemoved {
        feature: &'static str,
        profile: Profile,
    },
    InvalidProgram {
        reason: &'static str,
    },
}

impl Error {
    #[inline]
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            pc: None,
            location: None,
            location_suppressed: false,
        }
    }

    #[inline]
    pub fn at_pc(mut self, pc: usize) -> Self {
        if self.pc.is_none() && !self.location_suppressed {
            self.pc = Some(pc);
        }
        self
    }

    #[inline]
    pub fn at_source(mut self, source: &str, line: u32) -> Self {
        if self.location.is_none() && !self.location_suppressed {
            self.location = Some(ErrorLocation {
                source: source.to_owned(),
                line,
            });
        }
        self
    }

    #[inline]
    pub fn without_source_location(mut self) -> Self {
        self.location = None;
        self.location_suppressed = true;
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = &self.location {
            write!(formatter, "{}:{}: ", location.source, location.line)?;
        } else if let Some(pc) = self.pc {
            write!(formatter, "pc {pc}: ")?;
        }
        match &self.kind {
            ErrorKind::FuelExhausted { limit } => {
                write!(
                    formatter,
                    "instruction fuel exhausted after {limit} instructions"
                )
            }
            ErrorKind::RecursionLimit { limit } => {
                write!(
                    formatter,
                    "stack overflow (recursion limit exceeded at depth {limit})"
                )
            }
            ErrorKind::RegisterOutOfBounds { register, max } => {
                write!(
                    formatter,
                    "register r{register} is outside register file size {max}"
                )
            }
            ErrorKind::ConstantOutOfBounds { index, len } => {
                write!(
                    formatter,
                    "constant K{index} is outside constant pool size {len}"
                )
            }
            ErrorKind::TypeError { expected } => {
                write!(formatter, "type error: expected {expected}")
            }
            ErrorKind::OutOfMemory => formatter.write_str("not enough memory"),
            ErrorKind::TableIndexNil => formatter.write_str("table index is nil"),
            ErrorKind::TableIndexNaN => formatter.write_str("table index is NaN"),
            ErrorKind::InvalidKeyToNext => formatter.write_str("invalid key to 'next'"),
            ErrorKind::AssertionFailed { message } => formatter.write_str(message),
            ErrorKind::RuntimeError { message } => formatter.write_str(message),
            ErrorKind::GuestError { message, .. } => formatter.write_str(message),
            ErrorKind::Unsupported { feature } => {
                write!(formatter, "unsupported feature: {feature}")
            }
            ErrorKind::CapabilityRemoved { feature, profile } => {
                write!(formatter, "{feature} is not available in {profile:?}")
            }
            ErrorKind::InvalidProgram { reason } => write!(formatter, "invalid bytecode: {reason}"),
        }
    }
}

impl std::error::Error for Error {}
