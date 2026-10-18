//! Shared synchronous callback definitions for live values and persistent configuration.

use crate::function::{adapter::Token, IntoHostFunction};
use crate::invocation::{
    IntoEffectFunction, IntoExternalFunction, IntoInputFunction, IntoSlotReadFunction,
    IntoSlotWriteFunction,
};
use crate::{Error, Function, HostSchema, OpCapability, Result, Rustia, VmError, VmErrorKind};
use rustia_vm::HostFunctionOp;
use std::marker::PhantomData;

/// A Rust callback and its declared capabilities and fuel cost.
///
/// Pass it to [`Rustia::create_function`] to create a live function, or to
/// [`Rustia::register_function`] to keep the registration across decision resets.
/// It can also be added to a [`crate::HostModule`]. Plain closures work for
/// simple callbacks; use this type to choose capabilities or change the default
/// cost of one fuel unit per call.
///
/// ```
/// use rustia::{HostFunction, Rustia};
///
/// let mut lua = Rustia::builder().build()?;
/// lua.register_function("double", HostFunction::pure(|n: i64| n * 2).fuel_cost(2))?;
/// assert_eq!(lua.load("return double(21)").eval::<i64>()?, 42);
/// # Ok::<(), rustia::Error>(())
/// ```
///
/// Arguments and tuple results through arity eight can mix ordinary Rust data
/// with live [`crate::Value`], [`crate::Table`], and [`Function`] handles. Calls
/// are synchronous and receive no VM borrow, so they cannot call back into Luau.
/// A callback can return [`Result`] to report a failure.
///
/// Closures must be `Send + Sync + 'static`; use the capability-specific
/// constructors to borrow per-invocation host state. Captured allocations are
/// outside the VM memory limit. Captured state must honor the declared capability,
/// and captured destructors must not panic. In unwinding builds, a callback panic
/// is caught and blocks further guest execution until [`Rustia::reset_decision`];
/// in aborting builds, it terminates or traps instead.
pub struct HostFunction<C: HostSchema = ()> {
    build: Box<dyn FnOnce(String, u32) -> HostFunctionOp + Send + Sync>,
    fuel_cost: u32,
    // The erased dispatcher captures this exact schema's TypeId.
    schema: PhantomData<fn(C) -> C>,
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

    /// Declares a callback that neither reads nor changes external mutable state.
    ///
    /// Rustia trusts this declaration; it cannot inspect the closure's captures.
    pub fn pure<F, A, R>(function: F) -> Self
    where
        F: IntoHostFunction<A, R>,
    {
        Self::with_capability(OpCapability::Pure, function)
    }

    /// Declares the capability used by a callback's captured state.
    ///
    /// For a typed host schema, prefer constructors such as [`input`](Self::input)
    /// and [`effect`](Self::effect), which provide the appropriate borrowed view.
    pub fn with_capability<F, A, R>(capability: OpCapability, function: F) -> Self
    where
        F: IntoHostFunction<A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function.into_host_function(Token, name, fuel, capability.to_core())
        })
    }

    /// Creates a callback that borrows the invocation's deterministic input.
    pub fn input<F, A, R>(function: F) -> Self
    where
        F: IntoInputFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function.into_input_function(Token, name, fuel).into_op()
        })
    }

    /// Creates a callback that borrows the invocation's deterministic slot state.
    pub fn slot_read<F, A, R>(function: F) -> Self
    where
        F: IntoSlotReadFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function
                .into_slot_read_function(Token, name, fuel)
                .into_op()
        })
    }

    /// Creates a callback that stages effects in the invocation's record journal.
    pub fn effect<F, A, R>(function: F) -> Self
    where
        F: IntoEffectFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function.into_effect_function(Token, name, fuel).into_op()
        })
    }

    /// Creates a callback that stages slot writes in the invocation's record journal.
    pub fn slot_write<F, A, R>(function: F) -> Self
    where
        F: IntoSlotWriteFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function
                .into_slot_write_function(Token, name, fuel)
                .into_op()
        })
    }

    /// An external-state callback, rejected by deterministic profiles.
    pub fn external<F, A, R>(function: F) -> Self
    where
        F: IntoExternalFunction<C, A, R>,
    {
        Self::from_factory(move |name, fuel| {
            function
                .into_external_function(
                    Token,
                    name,
                    fuel,
                    OpCapability::ReadsExternalMutableState.to_core(),
                )
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
                .into_external_function(
                    Token,
                    name,
                    fuel,
                    OpCapability::NondeterministicEffect.to_core(),
                )
                .into_op()
        })
    }

    /// Sets the fuel charged for each call. The default is one; zero is rejected
    /// when the function is created or registered.
    pub fn fuel_cost(mut self, fuel_cost: u32) -> Self {
        self.fuel_cost = fuel_cost;
        self
    }

    pub(crate) fn into_op(self, name: String) -> HostFunctionOp {
        (self.build)(name, self.fuel_cost)
    }
}

mod adapter {
    use super::*;

    pub(crate) trait Definition<C: HostSchema, Signature> {
        fn into_definition(self, token: Token) -> HostFunction<C>;
    }
}

/// A plain callback or configured [`HostFunction`] accepted by Rustia.
///
/// This sealed trait can be used as a generic bound. Closures and function
/// pointers implement it automatically when their arguments and results are
/// supported by Rustia's callback conversions.
///
/// ```compile_fail
/// struct CustomDefinition;
/// impl rustia::IntoHostFunctionDefinition<(), ()> for CustomDefinition {}
/// ```
#[allow(private_bounds)]
pub trait IntoHostFunctionDefinition<C: HostSchema, Signature>:
    adapter::Definition<C, Signature>
{
}

impl<C: HostSchema, Signature, F> IntoHostFunctionDefinition<C, Signature> for F where
    F: adapter::Definition<C, Signature>
{
}

impl<C: HostSchema> adapter::Definition<C, ()> for HostFunction<C> {
    fn into_definition(self, _token: Token) -> HostFunction<C> {
        self
    }
}

impl<C, F, A, R> adapter::Definition<C, (A, R)> for F
where
    C: HostSchema,
    F: IntoHostFunction<A, R>,
{
    fn into_definition(self, _token: Token) -> HostFunction<C> {
        HostFunction::pure(self)
    }
}

impl<C: HostSchema> Rustia<C> {
    /// Creates a live Rust function without installing a global binding.
    ///
    /// Script references keep the callback alive after its Rust handle is
    /// dropped. Collection can release it once all references are gone. Decision
    /// reset invalidates every handle and releases the VM's ownership of the
    /// callback. Hardened leaves reject this constructor.
    ///
    /// Use [`register_function`](Self::register_function) for a global function
    /// that should be restored after reset.
    pub fn create_function<F, S>(&mut self, function: F) -> Result<Function>
    where
        F: IntoHostFunctionDefinition<C, S>,
    {
        self.create_named_function("<host>", function)
    }

    /// Creates a live Rust function with a name used in error reports.
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
        let function = function.into_definition(Token).into_op(debug_name.into());
        let value = self
            .vm
            .create_retained_function(function)
            .map_err(Error::from_engine)?;
        Ok(Function::new(value, self.vm.decision_epoch()))
    }

    /// Installs a global Rust function and restores its binding after decision reset.
    ///
    /// Replacing the registration releases the old configuration's ownership.
    /// Existing script references to the old function still work until reset.
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
            .install_host_function(function.into_definition(Token).into_op(name))
            .map_err(Error::from_engine)
    }
}
