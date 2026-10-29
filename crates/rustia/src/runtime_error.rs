//! Runtime errors independent of interpreter implementation types.

use crate::{Diagnostic, Profile, SourceLocation, SourceSpan};
use std::{fmt, sync::Arc};

/// A script execution, resource-limit, or live-handle failure.
///
/// Use [`kind`](Self::kind) for application decisions and
/// [`message`](Self::message) for the original message without a location prefix.
#[derive(Clone)]
pub struct VmError {
    pub(crate) kind: VmErrorKind,
    pub(crate) location: Option<RuntimeLocation>,
    pub(crate) location_suppressed: bool,
    pub(crate) diagnostic: Option<Arc<Diagnostic>>,
    pub(crate) span: Option<(u32, u32)>,
    core: rustia_vm::Error,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeLocation {
    pub(crate) source: Arc<str>,
    pub(crate) line: u32,
}

/// The category of a runtime failure.
///
/// Include a fallback arm: future versions may add categories. Script-thrown
/// objects are represented by their message here; their VM storage is private.
/// Match a kind instead of parsing the displayed message.
///
/// ```
/// use rustia::{Rustia, VmErrorKind};
///
/// let mut vm = Rustia::builder().fuel(1_000).build()?;
/// let error = vm.exec("while true do end").expect_err("the budget is finite");
/// assert!(matches!(
///     error.as_vm_error().map(|error| error.kind()),
///     Some(VmErrorKind::FuelExhausted { .. })
/// ));
/// // Start a fresh decision, discarding the failed script's guest state.
/// vm.reset_decision()?;
/// assert_eq!(vm.eval::<i32>("return 42")?, 42);
/// # Ok::<(), rustia::Error>(())
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VmErrorKind {
    /// The instruction fuel budget was exhausted. Script `pcall` cannot catch it.
    FuelExhausted {
        /// The exhausted budget, in instruction-fuel units rather than time.
        limit: u64,
    },
    /// Nested script calls reached the configured depth. Script `pcall` cannot catch it.
    RecursionLimit {
        /// The configured maximum script call depth.
        limit: usize,
    },
    /// Bytecode referred to a register outside its register file.
    RegisterOutOfBounds {
        /// The zero-based register index that was requested.
        register: usize,
        /// The number of available registers; valid indices are less than this value.
        max: usize,
    },
    /// Bytecode referred to a constant outside its constant pool.
    ConstantOutOfBounds {
        /// The zero-based constant index that was requested.
        index: usize,
        /// The number of available constants.
        len: usize,
    },
    /// An operation received a value of an incompatible type.
    TypeError {
        /// A description of the value type the operation expected.
        expected: &'static str,
    },
    /// A guest allocation failed or exceeded the VM's logical memory quota.
    /// This does not identify an operating-system memory limit.
    OutOfMemory,
    /// A typed callback was called without an invocation for its host schema.
    HostDispatcherUnavailable,
    /// A bounded host-invocation resource was exhausted.
    HostResourceExhausted {
        /// The resource name, such as a record-count or record-byte budget.
        resource: &'static str,
        /// The configured ceiling, in the units identified by `resource`.
        limit: u64,
    },
    /// A host callback's unwinding panic was contained and the VM was poisoned.
    /// Reset the decision before executing more guest code.
    HostPanicked {
        /// The callback name reported at the host boundary.
        function: String,
    },
    /// Execution was rejected after an earlier host panic.
    /// [`crate::Rustia::reset_decision`] clears the poisoned state.
    VmPoisoned,
    /// A coroutine attempted to yield across a boundary that cannot suspend.
    YieldAcrossBoundary,
    /// Decision reset was requested while the interpreter was executing.
    DecisionResetWhileRunning,
    /// A persistent host configuration change was requested during execution or a guest savepoint.
    HostConfigurationWhileRunning,
    /// Decision reset could not advance the finite epoch counter.
    DecisionEpochExhausted,
    /// A retained guest handle was used after its decision epoch ended.
    StaleRetainedValue {
        /// The epoch in which the handle was created.
        retained_epoch: u64,
        /// The interpreter's current epoch.
        current_epoch: u64,
    },
    /// A retained guest handle was used with a different interpreter.
    RetainedValueFromDifferentVm,
    /// A table write used `nil` as its key.
    TableIndexNil,
    /// A table write used NaN as its key.
    TableIndexNaN,
    /// `next` received a key that is not valid for the supplied table.
    InvalidKeyToNext,
    /// A script assertion failed.
    AssertionFailed {
        /// The assertion's message.
        message: String,
    },
    /// An ordinary script operation failed with a textual message.
    RuntimeError {
        /// The message without an added source-location prefix.
        message: String,
    },
    /// A script raised an error value, represented here by its original text.
    /// The original guest object's identity does not cross the facade boundary.
    GuestError {
        /// The textual representation of the script's error value.
        message: String,
    },
    /// Execution requested a feature that the engine does not implement.
    Unsupported {
        /// The unsupported feature's name.
        feature: &'static str,
    },
    /// The selected profile does not permit a requested capability.
    CapabilityRemoved {
        /// The unavailable capability or service.
        feature: &'static str,
        /// The specific operation, when the failure identifies one.
        operation: Option<String>,
        /// The profile that rejected the request.
        profile: Profile,
    },
    /// The hardened-leaf environment does not permit a requested operation or setting.
    HardenedLeafViolation {
        /// The rejected operation or setting.
        feature: &'static str,
    },
    /// An operation tried to change the sealed hardened-leaf guest environment.
    GuestEnvironmentSealed,
    /// A bytecode or program-state invariant was invalid.
    InvalidProgram {
        /// A description of the violated bytecode invariant.
        reason: &'static str,
    },
}

impl VmError {
    /// Creates a typed runtime error, for example from a Rust callback.
    ///
    /// A script-thrown object has no transferable identity in Rust. Returning a
    /// captured [`GuestError`](VmErrorKind::GuestError) from a callback raises
    /// its original message as a string, including after reset or in another VM.
    pub fn new(kind: VmErrorKind) -> Self {
        let mut error = Self::from_core(rustia_vm::Error::new(kind.to_core()));
        error.kind = kind;
        error
    }

    /// Returns the failure category without parsing its message.
    pub fn kind(&self) -> &VmErrorKind {
        &self.kind
    }

    /// Returns the original message without a source-location prefix.
    pub fn message(&self) -> VmErrorMessage<'_> {
        VmErrorMessage(self)
    }

    /// Returns the source label and one-based line, when available.
    pub fn source_location(&self) -> Option<SourceLocation<'_>> {
        if self.location_suppressed {
            return None;
        }
        self.location.as_ref().map(|location| SourceLocation {
            source: &location.source,
            line: location.line,
        })
    }

    /// Returns the source byte range, when available.
    pub fn source_span(&self) -> Option<SourceSpan> {
        self.span.map(|(start, end)| SourceSpan {
            start: start as usize,
            end: end as usize,
        })
    }

    /// Omits a source prefix from this failure's short message.
    ///
    /// Host-side reports may still identify the script that called a callback.
    pub fn without_source_location(self) -> Self {
        let mut error = Self::from_core(self.core.without_source_location());
        error.kind = self.kind;
        error
    }

    pub(crate) fn from_core(core: rustia_vm::Error) -> Self {
        Self {
            kind: VmErrorKind::from_core(&core.kind),
            location: core.location.as_ref().map(|location| RuntimeLocation {
                source: location.source.clone(),
                line: location.line,
            }),
            location_suppressed: core.location_suppressed,
            diagnostic: core
                .diagnostic
                .as_ref()
                .map(|diagnostic| Arc::new(Diagnostic::from_core(diagnostic))),
            span: core.span,
            core,
        }
    }

    pub(crate) fn into_core(mut self) -> rustia_vm::Error {
        if let rustia_vm::ErrorKind::GuestError { message, .. } = &self.core.kind {
            // Facade errors can outlive their VM or epoch. A raw guest object
            // cannot safely cross back into a VM without ownership and a root.
            self.core.kind = rustia_vm::ErrorKind::RuntimeError {
                message: message.clone(),
            };
        }
        self.core
    }

    pub(crate) fn is_transaction_abort(&self) -> bool {
        self.core.is_transaction_abort()
    }
}

/// A runtime error's original message, without a source-location prefix.
pub struct VmErrorMessage<'a>(&'a VmError);

impl fmt::Display for VmErrorMessage<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.core.message().fmt(formatter)
    }
}

impl fmt::Debug for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmError")
            .field("kind", &self.kind)
            .field("location", &self.source_location())
            .field("span", &self.source_span())
            .field("diagnostic", &self.diagnostic)
            .finish()
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.core.fmt(formatter)
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.host_cause.as_deref())
            .map(|cause| cause as &(dyn std::error::Error + 'static))
    }
}

impl VmErrorKind {
    fn from_core(kind: &rustia_vm::ErrorKind) -> Self {
        use rustia_vm::ErrorKind as Core;
        match kind {
            Core::FuelExhausted { limit } => Self::FuelExhausted { limit: *limit },
            Core::RecursionLimit { limit } => Self::RecursionLimit { limit: *limit },
            Core::RegisterOutOfBounds { register, max } => Self::RegisterOutOfBounds {
                register: *register,
                max: *max,
            },
            Core::ConstantOutOfBounds { index, len } => Self::ConstantOutOfBounds {
                index: *index,
                len: *len,
            },
            Core::TypeError { expected } => Self::TypeError { expected },
            Core::OutOfMemory => Self::OutOfMemory,
            Core::HostDispatcherUnavailable => Self::HostDispatcherUnavailable,
            Core::HostResourceExhausted { resource, limit } => Self::HostResourceExhausted {
                resource,
                limit: *limit,
            },
            Core::HostPanicked { function } => Self::HostPanicked {
                function: function.clone(),
            },
            Core::VmPoisoned => Self::VmPoisoned,
            Core::YieldAcrossBoundary => Self::YieldAcrossBoundary,
            Core::DecisionResetWhileRunning => Self::DecisionResetWhileRunning,
            Core::HostConfigurationWhileRunning => Self::HostConfigurationWhileRunning,
            Core::DecisionEpochExhausted => Self::DecisionEpochExhausted,
            Core::StaleRetainedValue {
                retained_epoch,
                current_epoch,
            } => Self::StaleRetainedValue {
                retained_epoch: *retained_epoch,
                current_epoch: *current_epoch,
            },
            Core::RetainedValueFromDifferentVm => Self::RetainedValueFromDifferentVm,
            Core::TableIndexNil => Self::TableIndexNil,
            Core::TableIndexNaN => Self::TableIndexNaN,
            Core::InvalidKeyToNext => Self::InvalidKeyToNext,
            Core::AssertionFailed { message } => Self::AssertionFailed {
                message: message.clone(),
            },
            Core::RuntimeError { message } => Self::RuntimeError {
                message: message.clone(),
            },
            Core::GuestError { message, .. } => Self::GuestError {
                message: message.clone(),
            },
            Core::Unsupported { feature } => Self::Unsupported { feature },
            Core::CapabilityRemoved {
                feature,
                operation,
                profile,
            } => Self::CapabilityRemoved {
                feature,
                operation: operation.clone(),
                profile: Profile::from_core(*profile),
            },
            Core::HardenedLeafViolation { feature } => Self::HardenedLeafViolation { feature },
            Core::GuestEnvironmentSealed => Self::GuestEnvironmentSealed,
            Core::InvalidProgram { reason } => Self::InvalidProgram { reason },
        }
    }

    fn to_core(&self) -> rustia_vm::ErrorKind {
        use rustia_vm::ErrorKind as Core;
        match self {
            Self::FuelExhausted { limit } => Core::FuelExhausted { limit: *limit },
            Self::RecursionLimit { limit } => Core::RecursionLimit { limit: *limit },
            Self::RegisterOutOfBounds { register, max } => Core::RegisterOutOfBounds {
                register: *register,
                max: *max,
            },
            Self::ConstantOutOfBounds { index, len } => Core::ConstantOutOfBounds {
                index: *index,
                len: *len,
            },
            Self::TypeError { expected } => Core::TypeError { expected },
            Self::OutOfMemory => Core::OutOfMemory,
            Self::HostDispatcherUnavailable => Core::HostDispatcherUnavailable,
            Self::HostResourceExhausted { resource, limit } => Core::HostResourceExhausted {
                resource,
                limit: *limit,
            },
            Self::HostPanicked { function } => Core::HostPanicked {
                function: function.clone(),
            },
            Self::VmPoisoned => Core::VmPoisoned,
            Self::YieldAcrossBoundary => Core::YieldAcrossBoundary,
            Self::DecisionResetWhileRunning => Core::DecisionResetWhileRunning,
            Self::HostConfigurationWhileRunning => Core::HostConfigurationWhileRunning,
            Self::DecisionEpochExhausted => Core::DecisionEpochExhausted,
            Self::StaleRetainedValue {
                retained_epoch,
                current_epoch,
            } => Core::StaleRetainedValue {
                retained_epoch: *retained_epoch,
                current_epoch: *current_epoch,
            },
            Self::RetainedValueFromDifferentVm => Core::RetainedValueFromDifferentVm,
            Self::TableIndexNil => Core::TableIndexNil,
            Self::TableIndexNaN => Core::TableIndexNaN,
            Self::InvalidKeyToNext => Core::InvalidKeyToNext,
            Self::AssertionFailed { message } => Core::AssertionFailed {
                message: message.clone(),
            },
            Self::RuntimeError { message } => Core::RuntimeError {
                message: message.clone(),
            },
            // A host-created error has no guest object to rethrow. Errors that
            // came from execution retain that object in VmError's private core.
            Self::GuestError { message } => Core::RuntimeError {
                message: message.clone(),
            },
            Self::Unsupported { feature } => Core::Unsupported { feature },
            Self::CapabilityRemoved {
                feature,
                operation,
                profile,
            } => Core::CapabilityRemoved {
                feature,
                operation: operation.clone(),
                profile: profile.to_core(),
            },
            Self::HardenedLeafViolation { feature } => Core::HardenedLeafViolation { feature },
            Self::GuestEnvironmentSealed => Core::GuestEnvironmentSealed,
            Self::InvalidProgram { reason } => Core::InvalidProgram { reason },
        }
    }
}
