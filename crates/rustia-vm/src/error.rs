use crate::profile::Profile;
use crate::value::Value;
use std::{fmt, sync::Arc};

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
    pub source: Arc<str>,
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
    HostDispatcherUnavailable,
    HostResourceExhausted {
        resource: &'static str,
        limit: u64,
    },
    HostPanicked {
        function: String,
    },
    VmPoisoned,
    YieldAcrossBoundary,
    DecisionResetWhileRunning,
    HostConfigurationWhileRunning,
    DecisionEpochExhausted,
    StaleRetainedValue {
        retained_epoch: u64,
        current_epoch: u64,
    },
    RetainedValueFromDifferentVm,
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
        operation: Option<String>,
        profile: Profile,
    },
    HardenedLeafViolation {
        feature: &'static str,
    },
    GuestEnvironmentSealed,
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
                source: Arc::from(source),
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

    /// Whether this failure must abort a transactional host invocation.
    #[doc(hidden)]
    #[inline]
    pub fn is_transaction_abort(&self) -> bool {
        self.is_always_hard_abort()
            || matches!(
                self.kind,
                ErrorKind::OutOfMemory | ErrorKind::YieldAcrossBoundary
            )
    }

    #[inline]
    pub(crate) fn is_always_hard_abort(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::FuelExhausted { .. }
                | ErrorKind::HostDispatcherUnavailable
                | ErrorKind::HostResourceExhausted { .. }
                | ErrorKind::HostPanicked { .. }
                | ErrorKind::VmPoisoned
        )
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
            ErrorKind::HostDispatcherUnavailable => {
                formatter.write_str("host callback requires an invocation with the matching host schema")
            }
            ErrorKind::HostResourceExhausted { resource, limit } => {
                write!(formatter, "host {resource} limit exhausted at {limit}")
            }
            ErrorKind::HostPanicked { function } => {
                write!(formatter, "host callback `{function}` panicked")
            }
            ErrorKind::VmPoisoned => {
                formatter.write_str("VM is poisoned by a prior host panic; reset it before reuse")
            }
            ErrorKind::YieldAcrossBoundary => {
                formatter.write_str("attempt to yield across metamethod/C-call boundary")
            }
            ErrorKind::DecisionResetWhileRunning => {
                formatter.write_str("decision reset requires an idle VM")
            }
            ErrorKind::HostConfigurationWhileRunning => {
                formatter.write_str("persistent host configuration requires an idle VM without a guest savepoint")
            }
            ErrorKind::DecisionEpochExhausted => {
                formatter.write_str("decision epoch counter exhausted")
            }
            ErrorKind::StaleRetainedValue {
                retained_epoch,
                current_epoch,
            } => write!(
                formatter,
                "retained value from decision epoch {retained_epoch} is stale; current decision epoch is {current_epoch}"
            ),
            ErrorKind::RetainedValueFromDifferentVm => {
                formatter.write_str("retained value belongs to a different VM")
            }
            ErrorKind::TableIndexNil => formatter.write_str("table index is nil"),
            ErrorKind::TableIndexNaN => formatter.write_str("table index is NaN"),
            ErrorKind::InvalidKeyToNext => formatter.write_str("invalid key to 'next'"),
            ErrorKind::AssertionFailed { message } => formatter.write_str(message),
            ErrorKind::RuntimeError { message } => formatter.write_str(message),
            ErrorKind::GuestError { message, .. } => formatter.write_str(message),
            ErrorKind::Unsupported { feature } => {
                write!(formatter, "unsupported feature: {feature}")
            }
            ErrorKind::CapabilityRemoved {
                feature,
                operation,
                profile,
            } => match operation {
                Some(operation) => {
                    write!(
                        formatter,
                        "{feature} `{operation}` is not available in {profile:?}"
                    )
                }
                None => write!(formatter, "{feature} is not available in {profile:?}"),
            },
            ErrorKind::HardenedLeafViolation { feature } => {
                write!(formatter, "{feature} is not available in the hardened leaf guest surface")
            }
            ErrorKind::GuestEnvironmentSealed => {
                formatter.write_str("hardened leaf guest environment is sealed")
            }
            ErrorKind::InvalidProgram { reason } => write!(formatter, "invalid bytecode: {reason}"),
        }
    }
}

impl std::error::Error for Error {}
