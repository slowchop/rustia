//! Shared synchronous callback definitions for live values and persistent configuration.

use crate::function::IntoHostFunction;
use crate::invocation::{
    IntoEffectFunction, IntoExternalFunction, IntoInputFunction, IntoSlotReadFunction,
    IntoSlotWriteFunction,
};
use crate::{Error, Function, HostSchema, OpCapability, Result, Rustia, VmError, VmErrorKind};
use rustia_vm::HostFunctionOp;
use std::marker::PhantomData;

/// A synchronous Rust callback definition, independent of a VM or decision epoch.
///
/// Use [`Rustia::create_function`] for a disposable live value or
/// [`Rustia::register_function`] for persistent host configuration. The same
/// definition can be added to a [`crate::HostModule`]. Capability-specific
/// constructors preserve the borrowed host-context and transactional effect rules.
///
/// Typed arguments and tuple results through arity eight can mix owned values
/// with rooted [`crate::Value`], [`crate::Table`], and [`Function`] handles. Owned
/// containers remain snapshots; live handles preserve graph identity. No Rust VM
/// borrow is supplied to the callback, so this API does not permit guest re-entry.
/// Callbacks must be `Send + Sync + 'static`; borrow invocation state through the
/// capability views. Captured destructors must not panic: callback execution is
/// panic-contained, but reclamation is not a guest finalizer or a fallible call.
/// Opaque captured allocations belong to the host, outside the VM memory quota.
pub struct HostFunction<C: HostSchema = ()> {
    build: Box<dyn FnOnce(String, u32) -> HostFunctionOp + Send + Sync>,
    fuel_cost: u32,
    schema: PhantomData<fn() -> C>,
}

impl<C: HostSchema> HostFunction<C> {
    fn from_factory(
        build: impl FnOnce(String, u32) -> HostFunctionOp + Send + Sync + 'static,
    ) -> Self {
        Self {
            build: Box::new(build),
            fuel_cost: 1,
            schema: PhantomData,
        }
    }

    /// A context-free callback with typed owned or live arguments and results.
    /// Purity of captured host state remains the embedder's responsibility.
    pub fn pure<F, A, R>(function: F) -> Self
    where
        F: IntoHostFunction<A, R>,
    {
        Self::with_capability(OpCapability::Pure, function)
    }

    /// A context-free callback with an explicit captured-host-state capability.
    /// Prefer capability-specific borrowed views when using a typed host schema.
    pub fn with_capability<F, A, R>(capability: OpCapability, function: F) -> Self
    where
        F: IntoHostFunction<A, R>,
    {
        Self::from_factory(move |name, fuel| function.into_host_function(name, fuel, capability))
    }

    /// A borrowed deterministic input reader.
    pub fn input<F, A, R>(function: F) -> Self
    where
        F: IntoInputFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| function.into_input_function(name, fuel).into_op())
    }

    /// A borrowed deterministic slot reader.
    pub fn slot_read<F, A, R>(function: F) -> Self
    where
        F: IntoSlotReadFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| function.into_slot_read_function(name, fuel).into_op())
    }

    /// A transactional effect recorder.
    pub fn effect<F, A, R>(function: F) -> Self
    where
        F: IntoEffectFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| function.into_effect_function(name, fuel).into_op())
    }

    /// A transactional slot-write recorder.
    pub fn slot_write<F, A, R>(function: F) -> Self
    where
        F: IntoSlotWriteFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function.into_slot_write_function(name, fuel).into_op()
        })
    }

    /// An external-state callback, rejected by deterministic profiles.
    pub fn external<F, A, R>(function: F) -> Self
    where
        F: IntoExternalFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function
                .into_external_function(name, fuel, OpCapability::ReadsExternalMutableState)
                .into_op()
        })
    }

    /// A nondeterministic callback, rejected by deterministic profiles.
    pub fn nondeterministic<F, A, R>(function: F) -> Self
    where
        F: IntoExternalFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function
                .into_external_function(name, fuel, OpCapability::NondeterministicEffect)
                .into_op()
        })
    }

    /// Sets the deterministic charge for each call. Zero is rejected at admission.
    pub fn fuel_cost(mut self, fuel_cost: u32) -> Self {
        self.fuel_cost = fuel_cost;
        self
    }

    pub(crate) fn into_op(self, name: String) -> HostFunctionOp {
        (self.build)(name, self.fuel_cost)
    }
}

/// Converts a plain typed callback or a configured [`HostFunction`] definition.
#[doc(hidden)]
pub trait IntoHostFunctionDefinition<C: HostSchema, Signature> {
    fn into_definition(self) -> HostFunction<C>;
}

impl<C: HostSchema> IntoHostFunctionDefinition<C, ()> for HostFunction<C> {
    fn into_definition(self) -> HostFunction<C> {
        self
    }
}

impl<C, F, A, R> IntoHostFunctionDefinition<C, (A, R)> for F
where
    C: HostSchema,
    F: IntoHostFunction<A, R>,
{
    fn into_definition(self) -> HostFunction<C> {
        HostFunction::pure(self)
    }
}

impl<C: HostSchema> Rustia<C> {
    /// Creates a disposable rooted function in the current VM decision.
    ///
    /// Guest references keep the callback alive after this handle is dropped.
    /// Collection releases unreachable callbacks; reset invalidates old handles
    /// and releases decision-local callback ownership. Hardened leaves reject
    /// this constructor. Use `register_function` for reset-persistent configuration.
    pub fn create_function<F, S>(&mut self, function: F) -> Result<Function>
    where
        F: IntoHostFunctionDefinition<C, S>,
    {
        self.create_named_function("<host>", function)
    }

    /// Creates a disposable function with an explicit diagnostic name.
    /// The name does not install a binding or change callback ownership.
    pub fn create_named_function<F, S>(
        &mut self,
        debug_name: impl Into<String>,
        function: F,
    ) -> Result<Function>
    where
        F: IntoHostFunctionDefinition<C, S>,
    {
        self.ensure_healthy()?;
        if self.vm.settings().guest_surface.is_hardened_leaf() {
            return Err(Error::from(
                VmError::new(VmErrorKind::HardenedLeafViolation {
                    feature: "live function construction",
                })
                .without_source_location(),
            ));
        }
        let function = function.into_definition().into_op(debug_name.into());
        let value = self
            .vm
            .create_retained_function(function)
            .map_err(Error::from)?;
        Ok(Function::new(value, self.vm.decision_epoch()))
    }

    /// Registers a reset-persistent callback under a global binding.
    /// Replacing a registration releases its old configuration ownership, while
    /// reachable guest copies of the old function remain independently callable.
    /// Registration requires an idle VM without an active guest-state savepoint.
    pub fn register_function<F, S>(&mut self, name: impl Into<String>, function: F) -> Result<()>
    where
        F: IntoHostFunctionDefinition<C, S>,
    {
        self.ensure_healthy()?;
        let name = name.into();
        if name.is_empty() {
            return Err(Error::embed(
                "host function name must not be empty".to_owned(),
            ));
        }
        self.vm
            .install_host_function(function.into_definition().into_op(name))
            .map_err(Error::from)
    }
}
