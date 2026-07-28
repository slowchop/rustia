mod extension_api;
mod function;
mod invocation;
mod macros;
mod serde_value;
mod value;

use function::IntoHostFunction;
use invocation::{HostBindings, TypedHostDispatcher};
use rustia_compiler::{compile_with_options, CompileError, CompileOptions};
use rustia_parser::SourceError;
use rustia_vm::{HardenedLeafProgram, Program, Vm};
use std::fmt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

pub use extension_api::{
    RustiaBuilder, RustiaExecutionProfile, RustiaExtension, RustiaHostFuelPolicy,
    RustiaHostFunctionInfo, ServiceClock, RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
    RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY, RUSTIA_MIN_HOST_FUNCTION_FUEL,
};
pub use function::{Exports, Function};
pub use invocation::{
    Effects, HostContext, HostModule, HostMut, HostSchema, Input, IntoEffectFunction,
    IntoExternalFunction, IntoInputFunction, IntoSlotReadFunction, IntoSlotWriteFunction,
    Invocation, InvocationLimits, InvocationOutcome, InvocationReceipt, InvocationUsage,
    SlotWrites, Slots,
};
pub use rustia_vm::{
    Error as VmError, ErrorKind as VmErrorKind, GuestSurface, OpCapability, Profile, VmMemoryStats,
    VmSettings, DECISION_RESET_ABI_VERSION as RUSTIA_DECISION_RESET_ABI_VERSION,
    HARDENED_LEAF_GUEST_ABI_VERSION as RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION,
};
pub use serde_value::SerdeValueError;
pub use value::{
    FromRustia, FromRustiaMulti, IntoRustia, IntoRustiaMulti, OwnedKey, OwnedTable, OwnedValue,
};

/// Safe Rust facade over one Luau VM.
///
/// `Rustia<()>` is the context-free compatibility surface. `Rustia<H>` stores
/// only the lifetime-independent [`HostSchema`] and registered callbacks; each
/// guest execution borrows a fresh [`HostContext<H>`] through
/// [`invoke`](Self::invoke) or [`invoke_into`](Self::invoke_into).
pub struct Rustia<C: HostSchema = ()> {
    vm: Vm,
    generation: u64,
    bindings: HostBindings<C>,
    journal: Vec<C::Record>,
}

impl<C> Rustia<C>
where
    C: HostSchema,
{
    fn from_profile(profile: Profile) -> Self {
        Self {
            vm: Vm::new(profile),
            generation: 0,
            bindings: HostBindings::default(),
            journal: Vec::new(),
        }
    }

    fn from_settings(profile: Profile, settings: VmSettings) -> Self {
        Self {
            vm: Vm::with_settings(profile, settings),
            generation: 0,
            bindings: HostBindings::default(),
            journal: Vec::new(),
        }
    }

    /// Returns the VM-owned logical live/reserved memory census and optional
    /// instruction-boundary peaks.
    pub fn memory_stats(&mut self) -> VmMemoryStats {
        self.vm.memory_stats()
    }

    /// Returns the typed core execution semantics read from this live VM.
    pub fn execution_profile(&self) -> RustiaExecutionProfile {
        RustiaExecutionProfile::from_vm(self.vm.profile(), self.vm.settings().guest_surface)
    }

    /// Returns the complete settings read from this live VM.
    ///
    /// This lets an embedder attest separately versioned budgets and services
    /// after construction rather than trusting only caller-side builder data.
    pub const fn vm_settings(&self) -> &VmSettings {
        self.vm.settings()
    }

    /// Current decision epoch. Epoch zero begins at construction.
    pub fn decision_epoch(&self) -> u64 {
        self.vm.decision_epoch()
    }

    /// Discards all guest state and begins a fresh decision epoch.
    ///
    /// VM configuration, services, extensions, and installed host functions
    /// persist. Globals inserted with [`insert_global`](Self::insert_global)
    /// and [`insert_serde`](Self::insert_serde) are decision inputs and must be
    /// rebound afterward. Every prior [`Function`] and [`Exports`] handle is
    /// invalidated, while host-owned [`CompiledModule`] and [`CompiledEntry`]
    /// values remain reusable. A hardened-leaf VM preserves its sealed
    /// immutable environment in place and discards every decision-local
    /// allocation after that seal.
    ///
    /// Replay protocols must record
    /// [`RUSTIA_DECISION_RESET_ABI_VERSION`].
    pub fn reset_decision(&mut self) -> Result<()> {
        self.vm.reset_decision().map_err(Error::Vm)?;
        self.journal.clear();
        Ok(())
    }

    /// Runs one typed, fuel-bounded host invocation and exposes its committed
    /// deterministic records from Rustia's reusable journal.
    ///
    /// Failure rolls back staged records, not ordinary guest heap/global
    /// mutation. Call [`reset_decision`](Self::reset_decision) when the host
    /// requires a fresh guest-state decision boundary.
    ///
    /// A panic in `operation` itself is ordinary caller-side Rust control
    /// flow: the private journal is dropped without publication and the panic
    /// continues unwinding; it is not translated into `HostPanicked` or VM
    /// poisoning. Panics from callbacks invoked by the VM are contained and
    /// poison the VM.
    pub fn invoke<'rustia, Return, Func>(
        &'rustia mut self,
        context: &mut dyn HostContext<C>,
        limits: InvocationLimits,
        operation: Func,
    ) -> Result<InvocationOutcome<'rustia, Return, C::Record>>
    where
        Func: for<'call> FnOnce(&mut Invocation<'call, C>) -> Result<Return>,
    {
        let (value, usage) = self.invoke_core(context, limits, operation)?;
        Ok(InvocationOutcome {
            value,
            records: &self.journal,
            usage,
        })
    }

    /// Runs one typed invocation and publishes its committed records into a
    /// caller-owned lane. Capacity is reserved before guest execution, so the
    /// final append cannot fail or allocate.
    pub fn invoke_into<Return, Func>(
        &mut self,
        context: &mut dyn HostContext<C>,
        limits: InvocationLimits,
        records: &mut Vec<C::Record>,
        operation: Func,
    ) -> Result<InvocationReceipt<Return>>
    where
        Func: for<'call> FnOnce(&mut Invocation<'call, C>) -> Result<Return>,
    {
        self.ensure_healthy()?;
        let reserve = usize::try_from(limits.max_records()).map_err(|_| {
            Error::Vm(VmError::new(VmErrorKind::HostResourceExhausted {
                resource: "effect-record count",
                limit: u64::from(limits.max_records()),
            }))
        })?;
        records
            .try_reserve(reserve)
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))?;
        let (value, usage) = self.invoke_core(context, limits, operation)?;
        records.append(&mut self.journal);
        Ok(InvocationReceipt { value, usage })
    }

    /// Executes one precompiled stateless entry, publishes its committed
    /// records, and resets the decision epoch before returning.
    ///
    /// Reset runs after success, returned failure, and an unwinding panic from
    /// guest-value conversion or a contained host callback. The original panic
    /// resumes after reset. If reset itself fails after a normal return, any
    /// records published by the invocation are removed from the caller lane
    /// and the reset error wins. `panic = "abort"` builds cannot unwind and
    /// therefore cannot provide panic containment. This is the complete
    /// authoritative-leaf lifecycle used by worker lanes.
    pub fn invoke_entry_into<Return>(
        &mut self,
        context: &mut dyn HostContext<C>,
        limits: InvocationLimits,
        records: &mut Vec<C::Record>,
        entry: &CompiledEntry,
    ) -> Result<InvocationReceipt<Return>>
    where
        Return: FromRustia,
    {
        let records_checkpoint = records.len();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.invoke_into(context, limits, records, |invocation| {
                invocation.call_entry::<Return>(entry)
            })
        }));
        let reset = self.reset_decision();
        finish_entry_lifecycle(records, records_checkpoint, outcome, reset)
    }

    fn invoke_core<Return, Func>(
        &mut self,
        context: &mut dyn HostContext<C>,
        limits: InvocationLimits,
        operation: Func,
    ) -> Result<(Return, InvocationUsage)>
    where
        Func: for<'call> FnOnce(&mut Invocation<'call, C>) -> Result<Return>,
    {
        self.ensure_healthy()?;
        self.journal.clear();
        let reserve = usize::try_from(limits.max_records()).map_err(|_| {
            Error::Vm(VmError::new(VmErrorKind::HostResourceExhausted {
                resource: "effect-record count",
                limit: u64::from(limits.max_records()),
            }))
        })?;
        self.journal
            .try_reserve(reserve)
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))?;

        let journal = std::mem::take(&mut self.journal);
        let dispatcher =
            TypedHostDispatcher::new(&self.bindings.handlers, context, journal, limits);
        let mut invocation =
            Invocation::new(&mut self.vm, &mut self.generation, dispatcher, limits);
        let result = operation(&mut invocation);
        let (dispatcher, fuel_used) = invocation.finish();
        let (journal, record_bytes, hard_error) = dispatcher.finish();
        self.journal = journal;
        if let Some(error) = hard_error {
            self.journal.clear();
            return Err(Error::Vm(error));
        }
        match result {
            Ok(value) => {
                let record_count = u32::try_from(self.journal.len()).unwrap_or(u32::MAX);
                Ok((
                    value,
                    InvocationUsage {
                        fuel_used,
                        record_count,
                        record_bytes,
                    },
                ))
            }
            Err(error) => {
                self.journal.clear();
                Err(error)
            }
        }
    }

    fn ensure_healthy(&self) -> Result<()> {
        if self.vm.is_poisoned() {
            Err(Error::Vm(VmError::new(VmErrorKind::VmPoisoned)))
        } else {
            Ok(())
        }
    }

    pub(crate) fn compile_and_run_raw(&mut self, source: &str) -> Result<rustia_vm::Value> {
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let program = rustia_compiler::compile(&chunk).map_err(Error::Compile)?;
        let value = self.vm.run(&program).map_err(Error::Vm)?;
        self.advance_generation();
        Ok(value)
    }

    pub(crate) fn compile_and_run_raw_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<rustia_vm::Value> {
        let module = Self::compile_module(source_name, source)?;
        let value = self.vm.run(&module.program).map_err(Error::Vm)?;
        self.advance_generation();
        Ok(value)
    }

    pub fn insert_global<T>(&mut self, name: impl AsRef<str>, value: T) -> Result<()>
    where
        T: IntoRustia,
    {
        let value = value.into_rustia();
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm.set_global_value(name, value).map_err(Error::Vm)
    }

    pub fn insert_serde<T>(&mut self, name: impl AsRef<str>, value: &T) -> Result<()>
    where
        T: serde::Serialize + ?Sized,
    {
        let value = OwnedValue::from_serialize(value)?;
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm.set_global_value(name, value).map_err(Error::Vm)
    }

    /// Registers a context-free `Fn(A, B, ...) -> R` with an explicit fuel
    /// charge. The registration method fixes the capability to `Pure`;
    /// `Result<R>` creates a fallible host call.
    pub fn add_pure<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::Embed(
                "typed host function name must not be empty".to_owned(),
            ));
        }
        let op = function.into_host_function(name, fuel_cost, OpCapability::Pure);
        self.vm.install_host_function(op).map_err(Error::Vm)
    }

    /// Registers a deterministic input reader with closure shape
    /// `for<'a> Fn(Input<'a, C>, A, B, ...) -> R`.
    pub fn add_input<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoInputFunction<C, Args, Return>,
    {
        let function = function.into_input_function(name.into(), fuel_cost);
        self.bindings.install(&mut self.vm, function)
    }

    /// Registers a deterministic slot reader with closure shape
    /// `for<'a> Fn(Slots<'a, C>, A, B, ...) -> R`.
    pub fn add_slot_read<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoSlotReadFunction<C, Args, Return>,
    {
        let function = function.into_slot_read_function(name.into(), fuel_cost);
        self.bindings.install(&mut self.vm, function)
    }

    /// Registers a deterministic effect recorder with closure shape
    /// `for<'a> Fn(Effects<'a, C>, A, B, ...) -> R`.
    pub fn add_effect<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoEffectFunction<C, Args, Return>,
    {
        let function = function.into_effect_function(name.into(), fuel_cost);
        self.bindings.install(&mut self.vm, function)
    }

    /// Registers a deterministic slot-write recorder with closure shape
    /// `for<'a> Fn(SlotWrites<'a, C>, A, B, ...) -> R`.
    pub fn add_slot_write<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoSlotWriteFunction<C, Args, Return>,
    {
        let function = function.into_slot_write_function(name.into(), fuel_cost);
        self.bindings.install(&mut self.vm, function)
    }

    /// Registers an external-state callback with closure shape
    /// `for<'a> Fn(HostMut<'a, C>, A, B, ...) -> R`.
    ///
    /// Deterministic profiles reject it before installation, and mutations
    /// through `HostMut` are outside Rustia's record rollback.
    pub fn add_external<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        let function = function.into_external_function(
            name.into(),
            fuel_cost,
            OpCapability::ReadsExternalMutableState,
        );
        self.bindings.install(&mut self.vm, function)
    }

    /// Registers an explicitly nondeterministic `HostMut` callback.
    ///
    /// Deterministic profiles reject it before installation.
    pub fn add_nondeterministic<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        let function = function.into_external_function(
            name.into(),
            fuel_cost,
            OpCapability::NondeterministicEffect,
        );
        self.bindings.install(&mut self.vm, function)
    }

    /// Installs a namespaced collection of capability-checked host functions.
    ///
    /// Installation is fail-closed: if validation or VM binding fails, no
    /// callable namespace or typed dispatcher bindings are retained.
    pub fn install_host_module(&mut self, module: HostModule<C>) -> Result<()> {
        module.install(&mut self.vm, &mut self.bindings)
    }

    pub fn compile_module(source_name: impl Into<String>, source: &str) -> Result<CompiledModule> {
        let source_name = source_name.into();
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let options = CompileOptions::default().with_source(source_name.clone(), source);
        let program = compile_with_options(&chunk, options).map_err(Error::Compile)?;
        Ok(CompiledModule {
            source_name,
            program,
        })
    }

    /// Compiles a chunk whose top-level body is invoked directly on each
    /// decision without installing exports or retaining bytecode in the VM.
    ///
    /// Stateless entries cannot contain nested function prototypes. Inputs and
    /// outputs should cross the typed host context, and the source should
    /// return its decision value directly.
    pub fn compile_entry(source_name: impl Into<String>, source: &str) -> Result<CompiledEntry> {
        let source_name = source_name.into();
        let chunk = rustia_parser::parse_source(source).map_err(Error::Source)?;
        let options = CompileOptions::default().with_source(source_name.clone(), source);
        let program = compile_with_options(&chunk, options).map_err(Error::Compile)?;
        let program = HardenedLeafProgram::new(program).map_err(Error::Vm)?;
        Ok(CompiledEntry {
            source_name,
            program,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    #[inline]
    pub(crate) fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    #[inline]
    pub(crate) fn vm(&self) -> &Vm {
        &self.vm
    }
}

fn finish_entry_lifecycle<T, Record>(
    records: &mut Vec<Record>,
    records_checkpoint: usize,
    outcome: std::thread::Result<Result<T>>,
    reset: Result<()>,
) -> Result<T> {
    match (outcome, reset) {
        (Ok(outcome), Ok(())) => outcome,
        (Ok(_), Err(reset_error)) => {
            records.truncate(records_checkpoint);
            Err(reset_error)
        }
        (Err(payload), _) => {
            records.truncate(records_checkpoint);
            resume_unwind(payload)
        }
    }
}

impl Rustia<()> {
    /// Returns the [`standard`](RustiaBuilder::standard) preset:
    /// reproducible-by-default, with a real stdout `print` sink on native
    /// targets.
    pub fn builder() -> RustiaBuilder {
        RustiaBuilder::standard()
    }

    pub fn new(profile: Profile) -> Self {
        Self::from_profile(profile)
    }

    pub fn with_settings(profile: Profile, settings: VmSettings) -> Self {
        Self::from_settings(profile, settings)
    }

    /// Creates a typed VM for host schema `H`.
    pub fn for_host<H>(profile: Profile) -> Rustia<H>
    where
        H: HostSchema,
    {
        Rustia::<H>::from_profile(profile)
    }

    /// Creates a typed VM for host schema `H` with explicit VM settings.
    pub fn for_host_with_settings<H>(profile: Profile, settings: VmSettings) -> Rustia<H>
    where
        H: HostSchema,
    {
        Rustia::<H>::from_settings(profile, settings)
    }

    pub fn compile(source_name: impl Into<String>, source: &str) -> Result<CompiledModule> {
        Self::compile_module(source_name, source)
    }

    pub fn compile_and_run(&mut self, source: &str) -> Result<Exports> {
        let value = self.compile_and_run_raw(source)?;
        let value = self.vm.retain_value(value);
        Ok(Exports::new(value, self.generation, self.decision_epoch()))
    }

    pub fn compile_and_run_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<Exports> {
        let value = self.compile_and_run_raw_named(source_name, source)?;
        let value = self.vm.retain_value(value);
        Ok(Exports::new(value, self.generation, self.decision_epoch()))
    }

    pub fn compile_and_run_exports(&mut self, source: &str) -> Result<Exports> {
        self.compile_and_run(source)
    }

    pub fn compile_and_run_exports_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<Exports> {
        self.compile_and_run_named(source_name, source)
    }

    pub fn run_owned(&mut self, source: &str) -> Result<OwnedValue> {
        let value = self.compile_and_run_raw(source)?;
        value::snapshot_value(&mut self.vm, value)
    }

    pub fn run_owned_named(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> Result<OwnedValue> {
        let value = self.compile_and_run_raw_named(source_name, source)?;
        value::snapshot_value(&mut self.vm, value)
    }

    pub fn run<T>(&mut self, source: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.run_owned(source)?;
        T::from_rustia(value)
    }

    pub fn run_named<T>(&mut self, source_name: impl Into<String>, source: &str) -> Result<T>
    where
        T: FromRustia,
    {
        let value = self.run_owned_named(source_name, source)?;
        T::from_rustia(value)
    }

    pub fn install(&mut self, module: CompiledModule) -> Result<Exports> {
        let value = self.vm.run(&module.program).map_err(Error::Vm)?;
        self.advance_generation();
        let value = self.vm.retain_value(value);
        Ok(Exports::new(value, self.generation, self.decision_epoch()))
    }

    pub fn hot_reload(&mut self, source_name: impl Into<String>, source: &str) -> Result<Exports> {
        let module = Self::compile_module(source_name, source)?;
        self.install(module)
    }

    pub fn add_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::Pure, function)
    }

    pub fn add_pure_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::Pure, function)
    }

    pub fn add_input_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsDeterministicInput, function)
    }

    pub fn add_slot_read_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsSlotState, function)
    }

    pub fn add_slot_write_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::WritesSlotEffect, function)
    }

    pub fn add_effect_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::EmitsDeterministicEffect, function)
    }

    pub fn add_external_state_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::ReadsExternalMutableState, function)
    }

    pub fn add_nondeterministic_fn<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.add_fn_with(name, 1, OpCapability::NondeterministicEffect, function)
    }

    pub fn add_fn_with<Func, Args, Return>(
        &mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        function: Func,
    ) -> Result<()>
    where
        Func: IntoHostFunction<Args, Return>,
    {
        let op = function.into_host_function(name.into(), fuel_cost, capability);
        self.vm.install_host_function(op).map_err(Error::Vm)
    }

    pub fn install_extension(&mut self, extension: RustiaExtension) -> Result<()> {
        extension.install(self)
    }
}

impl Default for Rustia {
    fn default() -> Self {
        Self::new(Profile::ReproducibleStandard)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Compiled artifact that can be cloned and installed into multiple VMs.
///
/// Constant and string storage is shared across clones; function and debug metadata is currently
/// cloned with the module.
#[derive(Clone)]
pub struct CompiledModule {
    source_name: String,
    program: Program,
}

impl CompiledModule {
    pub fn source_name(&self) -> &str {
        &self.source_name
    }
}

/// Precompiled top-level decision body for allocation-light repeated
/// invocation through [`Invocation::call_entry`].
#[derive(Clone)]
pub struct CompiledEntry {
    source_name: String,
    program: HardenedLeafProgram,
}

impl CompiledEntry {
    pub fn source_name(&self) -> &str {
        &self.source_name
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl From<rustia_lexer::Span> for SourceSpan {
    fn from(span: rustia_lexer::Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation<'a> {
    pub source: &'a str,
    pub line: u32,
}

#[derive(Debug)]
pub enum Error {
    Source(SourceError),
    Compile(CompileError),
    Vm(VmError),
    Embed(String),
}

impl Error {
    pub fn source_span(&self) -> Option<SourceSpan> {
        match self {
            Self::Source(SourceError::Lex(error)) => Some(error.span.into()),
            Self::Source(SourceError::Parse(error)) => Some(error.span.into()),
            Self::Compile(error) => error.span.map(Into::into),
            Self::Vm(_) | Self::Embed(_) => None,
        }
    }

    pub fn source_location(&self) -> Option<SourceLocation<'_>> {
        match self {
            Self::Vm(error) => error.location.as_ref().map(|location| SourceLocation {
                source: &location.source,
                line: location.line,
            }),
            Self::Source(_) | Self::Compile(_) | Self::Embed(_) => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::Compile(error) => error.fmt(formatter),
            Self::Vm(error) => error.fmt(formatter),
            Self::Embed(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::{
        finish_entry_lifecycle, Error, FromRustia, GuestSurface, OpCapability, OwnedTable,
        OwnedValue, Profile, Result, Rustia, RustiaBuilder, RustiaExecutionProfile,
        RustiaExtension, RustiaHostFunctionInfo, ServiceClock, SourceSpan, VmErrorKind, VmSettings,
        RUSTIA_DECISION_RESET_ABI_VERSION, RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
        RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY, RUSTIA_MIN_HOST_FUNCTION_FUEL,
    };
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::Vm;
    use serde::{Deserialize, Serialize};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::num::NonZeroU64;
    use std::rc::Rc;

    #[test]
    fn compile_and_run_smoke() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        let result = rustia
            .run_owned("local x: number = 6 * 7; return x")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 42.0);
    }

    #[test]
    fn entry_lifecycle_removes_new_records_when_reset_fails() {
        let mut records = vec![1, 2, 3];
        let result = finish_entry_lifecycle(
            &mut records,
            2,
            Ok(Ok(42)),
            Err(Error::Embed("reset failed".to_owned())),
        );
        assert!(matches!(result, Err(Error::Embed(ref message)) if message == "reset failed"));
        assert_eq!(records, [1, 2]);
    }

    #[test]
    fn compiled_modules_can_be_cloned_across_vms() {
        let module = Rustia::compile(
            "shared.luau",
            "local greeting = 'compiled string shared across VMs'; return { greeting = greeting }",
        )
        .unwrap();
        let mut first = Rustia::new(Profile::ReproducibleStandard);
        let mut second = Rustia::new(Profile::ReproducibleStandard);

        first.install(module.clone()).unwrap();
        second.install(module).unwrap();
    }

    #[test]
    fn typed_run_converts_owned_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia.run("return 40 + 2").unwrap();
        let numbers: Vec<i32> = rustia.run("return { 1, 2, 3 }").unwrap();
        let missing: Option<String> = rustia.run("return nil").unwrap();
        let named: BTreeMap<String, i32> = rustia.run("return { scout = 5, brute = 9 }").unwrap();

        assert_eq!(answer, 42.0);
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(missing, None);
        assert_eq!(named.get("scout"), Some(&5));
        assert_eq!(named.get("brute"), Some(&9));
    }

    #[test]
    fn typed_run_reports_conversion_errors() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia.run::<String>("return 42").unwrap_err().to_string();
        assert!(error.contains("expected string, got number"), "{error}");

        let array_error = rustia
            .run::<BTreeMap<String, i32>>("return { 1, 2 }")
            .unwrap_err()
            .to_string();
        assert!(
            array_error.contains("expected string-keyed table, got table with array entries"),
            "{array_error}"
        );

        let key_error = rustia
            .run::<BTreeMap<String, i32>>("return { [true] = 1 }")
            .unwrap_err()
            .to_string();
        assert!(
            key_error.contains("expected string-keyed table, got non-string key"),
            "{key_error}"
        );
    }

    #[test]
    fn numeric_table_keys_normalize_negative_zero_at_script_boundary() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local t = {}
t[-0] = 11
t[0] = t[0] + 31
return t[-0] == 42 and t[0] == 42 and rawget(t, -0) == 42 and rawget(t, 0) == 42
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn fuel_exhaustion_escapes_pcall_and_xpcall() {
        let cases = [
            "return pcall(function() while true do end end)",
            "return xpcall(function() while true do end end, function() return 'caught' end)",
        ];

        for source in cases {
            let settings = VmSettings {
                initial_fuel: Some(1_000),
                ..VmSettings::default()
            };
            let mut rustia = Rustia::with_settings(Profile::DeterministicSim, settings);
            let Err(Error::Vm(error)) = rustia.run_owned(source) else {
                panic!("expected fuel exhaustion to escape protected call: {source}");
            };
            assert!(matches!(
                error.kind,
                VmErrorKind::FuelExhausted { limit: 1_000 }
            ));
        }
    }

    #[test]
    fn owned_values_snapshot_returned_tables() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    label = 'spawn',
    count = 3,
    flags = { true, false },
}
",
            )
            .unwrap();
        let table = value.table().unwrap();
        assert_eq!(
            table.get_string("label"),
            Some(&OwnedValue::String("spawn".to_owned()))
        );
        assert_eq!(table.get_string("count"), Some(&OwnedValue::Number(3.0)));
        let flags = table.get_string("flags").unwrap().table().unwrap();
        assert_eq!(
            flags.array,
            vec![OwnedValue::Boolean(true), OwnedValue::Boolean(false)]
        );
    }

    #[test]
    fn owned_table_helpers_accept_typed_values_and_getters() {
        let labels = OwnedTable::string_entries([("kind", "scout"), ("role", "ai")]);
        assert_eq!(
            labels.get_string_as::<String>("kind").unwrap().as_deref(),
            Some("scout")
        );
        assert_eq!(labels.get_string_as::<String>("missing").unwrap(), None);

        let stats = OwnedTable::string_entries([("hp", 5_i32), ("speed", 7_i32)]);
        assert_eq!(stats.get_string_as::<i32>("hp").unwrap(), Some(5));

        let error = labels
            .get_string_as::<bool>("kind")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected boolean, got string"),
            "unexpected error: {error}"
        );

        let array = OwnedTable::array_values([1_i32, 2, 3]);
        assert_eq!(
            array.array,
            vec![
                OwnedValue::Number(1.0),
                OwnedValue::Number(2.0),
                OwnedValue::Number(3.0)
            ]
        );
        assert!(array.entries.is_empty());
    }

    #[test]
    fn owned_table_helpers_support_nested_typed_access() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    config = {
        spawn = {
            kind = 'scout',
            stats = { 5, 7 },
        },
    },
}
",
            )
            .unwrap();

        let root = value.table().unwrap();
        let config = root.get_string_table("config").unwrap().unwrap();
        let spawn = config.get_string_table("spawn").unwrap().unwrap();
        assert_eq!(
            spawn.get_string_as::<String>("kind").unwrap().as_deref(),
            Some("scout")
        );
        assert!(spawn.get_string_table("missing").unwrap().is_none());

        let stats = spawn.get_string_table("stats").unwrap().unwrap();
        assert_eq!(stats.get_array(0), Some(&OwnedValue::Number(5.0)));
        assert_eq!(stats.get_array_as::<i32>(0).unwrap(), Some(5));
        assert_eq!(stats.get_array_as::<i32>(1).unwrap(), Some(7));
        assert_eq!(stats.get_array_as::<i32>(2).unwrap(), None);

        let string_error = spawn.get_string_table("kind").unwrap_err().to_string();
        assert!(
            string_error.contains("expected table, got string"),
            "unexpected error: {string_error}"
        );
        let array_error = stats.get_array_table(0).unwrap_err().to_string();
        assert!(
            array_error.contains("expected table, got number"),
            "unexpected error: {array_error}"
        );
    }

    #[test]
    fn owned_table_helpers_deserialize_nested_fields() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct SpawnConfig<'a> {
            kind: &'a str,
            hp: u32,
            stats: Vec<u32>,
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct Enemy<'a> {
            kind: &'a str,
            hp: u32,
        }

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value = rustia
            .run_owned(
                "\
return {
    config = {
        kind = 'scout',
        hp = 5,
        stats = { 7, 9 },
    },
    enemies = {
        { kind = 'brute', hp = 9 },
        { kind = 'scout', hp = 5 },
    },
}
",
            )
            .unwrap();

        let root = value.table().unwrap();
        let config = root
            .get_string_deserialized::<SpawnConfig<'_>>("config")
            .unwrap()
            .unwrap();
        assert_eq!(
            config,
            SpawnConfig {
                kind: "scout",
                hp: 5,
                stats: vec![7, 9]
            }
        );
        assert!(root
            .get_string_deserialized::<SpawnConfig<'_>>("missing")
            .unwrap()
            .is_none());

        let enemies = root.get_string_table("enemies").unwrap().unwrap();
        assert_eq!(
            enemies.get_array_deserialized::<Enemy<'_>>(0).unwrap(),
            Some(Enemy {
                kind: "brute",
                hp: 9
            })
        );
        assert_eq!(
            enemies.get_array_deserialized::<Enemy<'_>>(1).unwrap(),
            Some(Enemy {
                kind: "scout",
                hp: 5
            })
        );
        assert!(enemies
            .get_array_deserialized::<Enemy<'_>>(2)
            .unwrap()
            .is_none());
    }

    #[test]
    fn insert_global_accepts_option_vec_and_owned_table() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let named_map = BTreeMap::from([("scout".to_owned(), 5_i32), ("brute".to_owned(), 9_i32)]);
        rustia.insert_global("maybe", Some(4_i32)).unwrap();
        rustia
            .insert_global("missing", Option::<i32>::None)
            .unwrap();
        rustia.insert_global("numbers", vec![1_i32, 2, 3]).unwrap();
        rustia.insert_global("named_map", named_map).unwrap();
        rustia
            .insert_global("named", OwnedTable::string_entries([("kind", "scout")]))
            .unwrap();

        let value = rustia
            .run_owned(
                "\
return maybe == 4
    and missing == nil
    and numbers[1] == 1
    and numbers[3] == 3
    and named_map.scout == 5
    and named_map.brute == 9
    and named.kind == 'scout'
",
            )
            .unwrap();
        assert!(bool::from_rustia(value).unwrap());
    }

    #[test]
    fn insert_global_and_typed_add_fn_are_callable() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        rustia.insert_global("difficulty", 2_i32).unwrap();
        rustia
            .add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))
            .unwrap();

        let result = rustia
            .run_owned("return clamp(12 + difficulty, 0, 10)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 10.0);
    }

    #[test]
    fn typed_host_functions_support_eight_arguments() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn(
                "sum8",
                |a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64| {
                    a + b + c + d + e + f + g + h
                },
            )
            .unwrap();

        let result = rustia
            .run_owned("return sum8(1, 2, 3, 4, 5, 6, 7, 8)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 36.0);
    }

    #[test]
    fn typed_host_functions_accept_and_return_owned_rust_shapes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("sum_vec", |values: Vec<f64>| {
                values.into_iter().sum::<f64>()
            })
            .unwrap();
        rustia
            .add_fn("maybe_name", |enabled: bool| {
                enabled.then_some("scout".to_owned())
            })
            .unwrap();
        rustia.add_fn("make_vec", || vec![2.0, 4.0, 6.0]).unwrap();
        rustia
            .add_fn("sum_map", |values: BTreeMap<String, f64>| {
                values.values().sum::<f64>()
            })
            .unwrap();
        rustia
            .add_fn("make_map", || {
                BTreeMap::from([("scout".to_owned(), 5.0), ("brute".to_owned(), 9.0)])
            })
            .unwrap();
        rustia
            .add_fn("make_table", || {
                OwnedTable::string_entries([("kind", "scout")])
            })
            .unwrap();

        let value = rustia
            .run_owned(
                "\
return sum_vec({ 1, 2, 3 }) == 6
    and maybe_name(true) == 'scout'
    and maybe_name(false) == nil
    and make_vec()[2] == 4
    and sum_map({ scout = 5, brute = 9 }) == 14
    and make_map().brute == 9
    and make_table().kind == 'scout'
",
            )
            .unwrap();
        assert!(bool::from_rustia(value).unwrap());
    }

    #[test]
    fn facade_capability_helpers_register_allowed_deterministic_functions() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        rustia.add_pure_fn("pure", || 5.0).unwrap();
        rustia.add_input_fn("input", || 2.0).unwrap();
        rustia.add_slot_read_fn("slot", || 3.0).unwrap();
        rustia
            .add_slot_write_fn("slot_write", |value: f64| value + 4.0)
            .unwrap();
        rustia
            .add_effect_fn("effect", |value: f64| value * 2.0)
            .unwrap();

        let result = rustia
            .run_owned("return pure() + input() + slot() + slot_write(1) + effect(3)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 21.0);
    }

    #[test]
    fn deterministic_facade_rejects_external_or_nondeterministic_helpers() {
        let mut external = Rustia::new(Profile::DeterministicSim);
        let Err(Error::Vm(error)) = external.add_external_state_fn("clock", || 1.0) else {
            panic!("expected deterministic profile to reject external-state host function");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let mut nondeterministic = Rustia::new(Profile::DeterministicSim);
        let Err(Error::Vm(error)) = nondeterministic.add_nondeterministic_fn("random", || 1.0)
        else {
            panic!("expected deterministic profile to reject nondeterministic host function");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn standard_facade_accepts_external_and_nondeterministic_helpers() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia.add_external_state_fn("clock", || 7.0).unwrap();
        rustia.add_nondeterministic_fn("random", || 11.0).unwrap();

        let result = rustia.run_owned("return clock() + random()").unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 18.0);
    }

    #[test]
    fn typed_host_function_arg_errors_include_function_and_position() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))
            .unwrap();

        let wrong_type = rustia.run_owned("return clamp('bad', 0, 10)").unwrap_err();
        let wrong_type = wrong_type.to_string();
        assert!(
            wrong_type.contains("host function `clamp` argument 1: expected number, got string"),
            "{wrong_type}"
        );

        let missing_arg = rustia.run_owned("return clamp(4, 0)").unwrap_err();
        let missing_arg = missing_arg.to_string();
        assert!(
            missing_arg.contains("host function `clamp` argument 3: expected number, got nil"),
            "{missing_arg}"
        );
    }

    #[test]
    fn typed_host_functions_can_return_rustia_result_errors() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("checked_div", |x: f64, y: f64| -> Result<f64> {
                if y == 0.0 {
                    return Err(Error::Embed("division by zero".to_owned()));
                }
                Ok(x / y)
            })
            .unwrap();

        let value: f64 = rustia.run("return checked_div(12, 3)").unwrap();
        assert_eq!(value, 4.0);

        let error = rustia
            .run_owned("return checked_div(12, 0)")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("host function `checked_div` return value: division by zero"),
            "{error}"
        );
    }

    #[test]
    fn exports_facade_retains_script_functions() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    label = 'ai',
    decide = function(x)
        return x + 1
    end,
}
",
            )
            .unwrap();
        assert_eq!(exports.get::<String>(&mut rustia, "label").unwrap(), "ai");

        let decide = exports.function(&mut rustia, "decide").unwrap();
        let result: f64 = decide.call(&mut rustia, (41.0,)).unwrap();
        assert_eq!(result, 42.0);
    }

    #[test]
    fn retained_script_functions_return_vec_and_option_shapes() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    values = function()
        return { 1, 2, 3 }
    end,
    maybe = function(enabled)
        if enabled then
            return 'scout'
        end
    end,
}
",
            )
            .unwrap();

        let values = exports.function(&mut rustia, "values").unwrap();
        let values: Vec<f64> = values.call(&mut rustia, ()).unwrap();
        assert_eq!(values, vec![1.0, 2.0, 3.0]);

        let maybe = exports.function(&mut rustia, "maybe").unwrap();
        let found: Option<String> = maybe.call(&mut rustia, (true,)).unwrap();
        let missing: Option<String> = maybe.call(&mut rustia, (false,)).unwrap();
        assert_eq!(found.as_deref(), Some("scout"));
        assert_eq!(missing, None);
    }

    #[test]
    fn retained_script_functions_support_eight_arguments_and_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    rotate = function(a, b, c, d, e, f, g, h)
        return h, g, f, e, d, c, b, a
    end,
}
",
            )
            .unwrap();

        let rotate = exports.function(&mut rustia, "rotate").unwrap();
        let result: (f64, f64, f64, f64, f64, f64, f64, f64) = rotate
            .call(&mut rustia, (1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0))
            .unwrap();
        assert_eq!(result, (8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0));
    }

    #[test]
    fn builder_extension_installs_namespaced_deterministic_ops() {
        let extension = RustiaExtension::new("host")
            .fn_slot_read("bias", || 2.0)
            .fn_effect("emit", |value: f64| value + 1.0);
        let mut rustia = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build()
            .unwrap();

        let result = rustia
            .run_owned("return host.bias() + host.emit(4)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 7.0);
    }

    #[test]
    fn extension_reports_host_function_metadata() {
        let extension = RustiaExtension::new("host")
            .fn_pure("zero", || 1.0)
            .fn_pure("try_zero", || -> Result<f64> { Ok(1.0) })
            .fn_pure("split", |value: f64| (value, value > 0.0))
            .fn_with(
                "sum3",
                7,
                OpCapability::ReadsSlotState,
                |a: f64, b: f64, c: f64| a + b + c,
            );

        assert_eq!(extension.namespace(), Some("host"));
        assert_eq!(extension.fuel_policy(), RUSTIA_HOST_FUEL_POLICY);
        assert_eq!(
            extension.fuel_policy().abi_version,
            RUSTIA_HOST_FUEL_ABI_VERSION
        );
        assert_eq!(
            extension.fuel_policy().default_function_fuel,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL
        );
        assert_eq!(
            extension.fuel_policy().minimum_function_fuel,
            RUSTIA_MIN_HOST_FUNCTION_FUEL
        );
        let functions = extension.functions().collect::<Vec<_>>();
        assert_eq!(
            functions,
            vec![
                RustiaHostFunctionInfo {
                    name: "zero",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 0,
                    parameter_types: &[],
                    return_types: &["f64"],
                    fallible: false,
                },
                RustiaHostFunctionInfo {
                    name: "try_zero",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 0,
                    parameter_types: &[],
                    return_types: &["f64"],
                    fallible: true,
                },
                RustiaHostFunctionInfo {
                    name: "split",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 1,
                    capability: OpCapability::Pure,
                    parameter_count: 1,
                    parameter_types: &["f64"],
                    return_types: &["f64", "bool"],
                    fallible: false,
                },
                RustiaHostFunctionInfo {
                    name: "sum3",
                    fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                    fuel_cost: 7,
                    capability: OpCapability::ReadsSlotState,
                    parameter_count: 3,
                    parameter_types: &["f64", "f64", "f64"],
                    return_types: &["f64"],
                    fallible: false,
                },
            ]
        );
    }

    #[test]
    fn extension_rejects_zero_fuel_host_functions() {
        let extension = RustiaExtension::new("host").fn_with("free", 0, OpCapability::Pure, || 1.0);
        let result = Rustia::builder().extension(extension).build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected zero-fuel host function to be rejected");
        };
        let VmErrorKind::InvalidProgram { reason } = error.kind else {
            panic!("expected invalid-program error, got {error:?}");
        };
        assert_eq!(reason, "host operation fuel cost must be non-zero");
    }

    #[test]
    fn deterministic_builder_rejects_nondeterministic_extension() {
        let extension = RustiaExtension::new("host").fn_nondeterministic("clock", || 1.0);
        let result = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject nondeterministic extension");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn deterministic_builder_rejects_external_state_extension() {
        let extension = RustiaExtension::new("host").fn_external_state("clock", || 1.0);
        let result = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject external-state extension");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn hot_reload_compile_error_keeps_old_function_callable() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .hot_reload("ai.luau", "return { decide = function() return 1 end }")
            .unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();
        let generation = rustia.generation();

        assert!(rustia.hot_reload("ai.luau", "return {").is_err());
        assert_eq!(rustia.generation(), generation);
        assert_eq!(decide.generation(), generation);
        let result: f64 = decide.call(&mut rustia, ()).unwrap();
        assert_eq!(result, 1.0);
    }

    #[test]
    fn retained_handles_are_versioned_across_successful_hot_reload() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let first_exports = rustia
            .hot_reload(
                "ai.luau",
                "return { label = 'old', decide = function() return 1 end }",
            )
            .unwrap();
        let first_generation = first_exports.generation();
        let first_decide = first_exports.function(&mut rustia, "decide").unwrap();

        let second_exports = rustia
            .hot_reload(
                "ai.luau",
                "return { label = 'new', decide = function() return 2 end }",
            )
            .unwrap();
        let second_generation = second_exports.generation();
        let second_decide = second_exports.function(&mut rustia, "decide").unwrap();

        assert!(second_generation > first_generation);
        assert_eq!(rustia.generation(), second_generation);
        assert_eq!(first_decide.generation(), first_generation);
        assert_eq!(second_decide.generation(), second_generation);
        assert!(!first_exports.is_current(&rustia));
        assert!(!first_decide.is_current(&rustia));
        assert!(second_exports.is_current(&rustia));
        assert!(second_decide.is_current(&rustia));
        let stale_exports = first_exports.ensure_current(&rustia).unwrap_err();
        assert!(
            stale_exports
                .to_string()
                .contains("stale exports handle from generation"),
            "{stale_exports}"
        );
        assert!(first_exports.value_current(&mut rustia).is_err());
        assert!(first_exports
            .get_current::<String>(&mut rustia, "label")
            .is_err());
        assert!(first_exports
            .function_current(&mut rustia, "decide")
            .is_err());
        assert_eq!(
            first_exports.get::<String>(&mut rustia, "label").unwrap(),
            "old"
        );
        assert_eq!(
            second_exports
                .get_current::<String>(&mut rustia, "label")
                .unwrap(),
            "new"
        );
        let current_decide = second_exports
            .function_current(&mut rustia, "decide")
            .unwrap();
        assert_eq!(current_decide.generation(), second_generation);

        let old_result: f64 = first_decide.call(&mut rustia, ()).unwrap();
        let new_result: f64 = second_decide.call(&mut rustia, ()).unwrap();
        assert_eq!(old_result, 1.0);
        assert_eq!(new_result, 2.0);

        let stale = first_decide
            .call_current::<_, f64>(&mut rustia, ())
            .unwrap_err();
        assert!(
            stale
                .to_string()
                .contains("stale function handle from generation"),
            "{stale}"
        );
        let current: f64 = second_decide.call_current(&mut rustia, ()).unwrap();
        assert_eq!(current, 2.0);
    }

    #[test]
    fn retained_function_root_survives_after_exports_are_dropped() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run(
                "local state = { answer = 42 }; \
                 return { read = function() return state.answer end }",
            )
            .unwrap();
        let read = exports.function(&mut rustia, "read").unwrap();
        drop(exports);

        let _: f64 = rustia.run("collectgarbage('collect'); return 0").unwrap();
        let answer: f64 = read.call(&mut rustia, ()).unwrap();
        assert_eq!(answer, 42.0);
    }

    #[test]
    fn retained_function_root_survives_repeated_slot_reuse() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run(
                "local state = { answer = 42 }; \
                 return { read = function() return state.answer end }",
            )
            .unwrap();
        let read = exports.function(&mut rustia, "read").unwrap();
        drop(exports);

        for _ in 0..32 {
            let _: f64 = rustia
                .run(
                    "for i = 1, 256 do \
                         local state = { answer = i }; \
                         local function read() return state.answer end \
                         if read() ~= i then error('bad churn closure') end \
                     end \
                     collectgarbage('collect'); \
                     return 0",
                )
                .unwrap();
            let answer: f64 = read.call(&mut rustia, ()).unwrap();
            assert_eq!(answer, 42.0);
        }
    }

    #[test]
    fn retained_handles_reject_a_different_vm() {
        let mut first = Rustia::new(Profile::ReproducibleStandard);
        let exports = first
            .compile_and_run("return { read = function() return 42 end }")
            .unwrap();
        let read = exports.function(&mut first, "read").unwrap();
        let mut second = Rustia::new(Profile::ReproducibleStandard);

        let exports_error = exports.value(&mut second).unwrap_err();
        assert!(
            exports_error
                .to_string()
                .contains("belongs to a different Rustia VM"),
            "{exports_error}"
        );
        let function_error = read.call::<_, f64>(&mut second, ()).unwrap_err();
        assert!(
            function_error
                .to_string()
                .contains("belongs to a different Rustia VM"),
            "{function_error}"
        );
    }

    #[test]
    fn named_compile_preserves_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia
            .hot_reload("bad_ai.luau", "local t = {}; t[0 / 0] = 1")
            .unwrap_err();
        assert!(error.to_string().contains("bad_ai.luau:1:"), "{error}");
    }

    #[test]
    fn named_run_helpers_preserve_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia.run_named("inline.luau", "return 40 + 2").unwrap();
        assert_eq!(answer, 42.0);

        let owned = rustia.run_owned_named("owned.luau", "return 'ok'").unwrap();
        assert_eq!(String::from_rustia(owned).unwrap(), "ok");

        let exports = rustia
            .compile_and_run_exports_named("exports.luau", "return { answer = 42 }")
            .unwrap();
        assert_eq!(exports.get::<i32>(&mut rustia, "answer").unwrap(), 42);

        let generation = rustia.generation();
        let error = rustia
            .run_named::<f64>("bad_inline.luau", "local t = {}; t[0 / 0] = 1")
            .unwrap_err();
        assert!(error.to_string().contains("bad_inline.luau:1:"), "{error}");
        assert_eq!(rustia.generation(), generation);
    }

    #[test]
    fn public_errors_expose_source_span_and_runtime_location() {
        let Err(error) = Rustia::compile("compile-diag.luau", "break") else {
            panic!("break outside loop should fail compilation");
        };
        assert_eq!(error.source_span(), Some(SourceSpan { start: 0, end: 5 }));
        assert_eq!(error.source_location(), None);

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let Err(error) = rustia.run_owned_named(
            "runtime-diag.luau",
            "\
local t = {}
t[0 / 0] = 1
",
        ) else {
            panic!("NaN table key should fail at runtime");
        };
        let location = error
            .source_location()
            .expect("runtime error should carry source location");
        assert_eq!(location.source, "runtime-diag.luau");
        assert_eq!(location.line, 2);
        assert_eq!(error.source_span(), None);
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct ScenarioSpec {
        name: String,
        difficulty: u32,
        enemies: Vec<EnemySpec>,
    }

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct EnemySpec {
        kind: String,
        hp: u32,
    }

    #[test]
    fn serde_deserializes_exports_and_inserts_globals() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .compile_and_run_exports(
                "\
return {
    name = 'arena',
    difficulty = 3,
    enemies = {
        { kind = 'scout', hp = 5 },
        { kind = 'brute', hp = 9 },
    },
}
",
            )
            .unwrap();
        let spec: ScenarioSpec = exports.deserialize(&mut rustia).unwrap();
        assert_eq!(
            spec,
            ScenarioSpec {
                name: "arena".to_owned(),
                difficulty: 3,
                enemies: vec![
                    EnemySpec {
                        kind: "scout".to_owned(),
                        hp: 5
                    },
                    EnemySpec {
                        kind: "brute".to_owned(),
                        hp: 9
                    },
                ],
            }
        );
        let current_spec: ScenarioSpec = exports.deserialize_current(&mut rustia).unwrap();
        assert_eq!(current_spec, spec);

        rustia.insert_serde("config", &spec).unwrap();
        let value = rustia
            .run_owned("return config.enemies[2].kind .. ':' .. tostring(config.difficulty)")
            .unwrap();
        assert_eq!(String::from_rustia(value).unwrap(), "brute:3");
    }

    #[test]
    fn rustia_extension_macro_builds_capability_tagged_extension() {
        let extension = crate::rustia_extension!("host" {
            pure(fuel = 2) fn fixed_mul(a: i32, b: i32) -> i32 {
                a * b
            }

            effect fn emit(value: f64) -> f64 {
                value + 10.0
            }
        });
        let mut rustia = Rustia::builder()
            .profile(Profile::DeterministicSim)
            .extension(extension)
            .build()
            .unwrap();

        let value = rustia
            .run_owned("return host.fixed_mul(6, 7) + host.emit(1)")
            .unwrap();
        assert_eq!(f64::from_rustia(value).unwrap(), 53.0);
    }

    #[test]
    fn debug_traceback_for_errored_coroutine_reports_top_frame() {
        let source = "\
function halp(key, value)
    local t = {}
    t[key] = value
    return t
end

local co = coroutine.create(halp)
local ok, err = coroutine.resume(co, 0 / 0, 42)
return tostring(ok) .. \"|\" .. tostring(err) .. \"|\" .. debug.traceback(co)
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let traceback = vm.string_value(result).unwrap().unwrap();
        assert_eq!(
            traceback,
            "false|debug.luau:3: table index is NaN|debug.luau:3 function halp\n"
        );
    }

    #[test]
    fn debug_info_line_defined_precedes_active_line() {
        let source = "\
function testlinedefined()
    local line = debug.info(1, \"l\")
    local linedefined = debug.info(testlinedefined, \"l\")
    return tostring(line) .. \"|\" .. tostring(linedefined)
end

return testlinedefined()
";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("debug.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        let lines = vm.string_value(result).unwrap().unwrap();
        assert_eq!(lines, "2|1");
    }

    #[test]
    fn conformance_int64_userdata_supports_metamethods_and_value_field() {
        let settings = VmSettings {
            enable_conformance_int64_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local v = int64(42)
v.value = 4
return tostring(v + 2) .. '|' .. tostring(v == int64(4)) .. '|' .. tostring(int64(-5) // 2)
",
            )
            .unwrap();
        assert_eq!(String::from_rustia(result).unwrap(), "6|true|-3");
    }

    #[test]
    fn conformance_native_userdata_supports_vec2_and_vertex_surface() {
        let settings = VmSettings {
            enable_conformance_native_userdata: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local s = vec2(5, 4)
local t = vec2(1, 2)
local u = vec2(10, 20)
local x = -u + s * t
local m = u:Min(s)
local v = vertex(vector.create(1, 2, 3), vector.create(0.5, 0, 0.5), vec2(0.25, 0.5))
return x.X == -5 and x.Y == -12 and s:Dot(t) == 13 and m.X == 5 and m.Y == 4 and v.uv.X == 0.25 and v.normal.Z == 0.5
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn conformance_native_type_checks_validate_annotated_parameters() {
        let settings = VmSettings {
            enable_conformance_native_type_checks: true,
            ..VmSettings::default()
        };
        let mut rustia = Rustia::with_settings(Profile::ReproducibleStandard, settings);
        let result = rustia
            .run_owned(
                "\
local function add(a: number, b: number)
    assert(is_native())
    return a + b
end
local function checkbuffer(a: buffer?) return a == nil or type(a) == 'buffer' end
local ok = pcall(add, nil, 2)
return add(1, 3) == 4 and not ok and checkbuffer(nil) and checkbuffer(buffer.create(1))
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn coroutine_resume_arguments_return_from_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co
local seen
local function body(a, ...)
    local arg = {...}
    for i = 1, #arg do
        seen = { coroutine.yield(table.unpack(arg[i])) }
    end
    return table.unpack(a)
end
co = coroutine.create(body)
local s, a, b, c, d = coroutine.resume(co, {1, 2, 3}, {}, {1}, {'a', 'b', 'c'})
local ok1 = s and a == nil and coroutine.status(co) == 'suspended'
s, a, b, c, d = coroutine.resume(co, 1, 2, 3)
local ok2 = s and #seen == 3 and seen[1] == 1 and seen[2] == 2 and seen[3] == 3
    and a == 1 and b == nil
s, a, b, c, d = coroutine.resume(co, 'done')
local ok3 = s and #seen == 1 and seen[1] == 'done'
    and a == 'a' and b == 'b' and c == 'c' and d == nil
s, a, b, c, d = coroutine.resume(co)
local ok4 = s and a == 1 and b == 2 and c == 3 and d == nil
    and coroutine.status(co) == 'dead'
return ok1 and ok2 and ok3 and ok4
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn coroutine_wrap_resumes_yield_call_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local f = coroutine.wrap(function()
    local a, b = coroutine.yield('yielded')
    return a, b
end)
local first = f()
local second, third = f('resume', 42)
return first == 'yielded' and second == 'resume' and third == 42
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn pcall_can_yield_and_resume_successfully() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield('pause')
        return 1, 2, 3
    end)
end)
local ok, value = coroutine.resume(co)
local first = ok and value == 'pause' and coroutine.status(co) == 'suspended'
local done, protected, a, b, c = coroutine.resume(co)
return first and done and protected and a == 1 and b == 2 and c == 3
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn nested_pcall_preserves_protected_results_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield(1)
        return pcall(function()
            coroutine.yield(2)
            return 3
        end)
    end)
end)
local ok, a = coroutine.resume(co)
local first = ok and a == 1
ok, a = coroutine.resume(co)
local second = ok and a == 2
local done, outer, inner, value = coroutine.resume(co)
return first and second and done and outer and inner and value == 3
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn pcall_catches_error_after_yield() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let result = rustia
            .run_owned(
                "\
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield()
        error('foo')
    end)
end)
local ok = coroutine.resume(co)
local done, protected, err = coroutine.resume(co)
return ok and done and not protected and string.find(err, 'foo', 1, true) ~= nil
",
            )
            .unwrap();
        assert!(bool::from_rustia(result).unwrap());
    }

    #[test]
    fn deterministic_builder_captures_print_through_the_stdout_sink() {
        let captured = Rc::new(RefCell::new(String::new()));
        let sink = Rc::clone(&captured);
        let mut rustia = RustiaBuilder::deterministic()
            .stdout(move |text| sink.borrow_mut().push_str(text))
            .build()
            .unwrap();

        rustia.run_owned("print('hello', 42)").unwrap();
        assert_eq!(captured.borrow().as_str(), "hello\t42\n");
    }

    #[test]
    fn execution_profiles_have_stable_unique_encodings() {
        let profiles = [
            (
                RustiaExecutionProfile::new(Profile::DeterministicSim, GuestSurface::HardenedLeaf),
                1,
                [1, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(Profile::LuauCompat, GuestSurface::Standard),
                2,
                [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(Profile::ReproducibleStandard, GuestSurface::Standard),
                3,
                [1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(Profile::DeterministicSim, GuestSurface::Standard),
                4,
                [1, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(Profile::LuauCompat, GuestSurface::HardenedLeaf),
                5,
                [1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(
                    Profile::ReproducibleStandard,
                    GuestSurface::HardenedLeaf,
                ),
                6,
                [1, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0],
            ),
        ];
        for (index, (profile, compatibility_id, identity_bytes)) in profiles.iter().enumerate() {
            assert_eq!(profile.compatibility_id(), *compatibility_id);
            assert_eq!(profile.identity_bytes(), *identity_bytes);
            for (other_profile, _, _) in &profiles[index + 1..] {
                assert_ne!(profile.compatibility_id(), other_profile.compatibility_id());
                assert_ne!(profile.identity_bytes(), other_profile.identity_bytes());
            }
        }
    }

    #[test]
    fn hardened_execution_profile_transfers_to_the_live_vm() {
        let expected = RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF;

        let builder = RustiaBuilder::deterministic_leaf().memory_limit_bytes(512 * 1024);
        assert_eq!(builder.execution_profile(), expected);
        assert_eq!(builder.vm_settings().memory_limit_bytes, Some(512 * 1024));
        assert!(!builder.vm_settings().enable_memory_tracking);
        let rustia = builder.build_for_host::<()>().unwrap();
        assert_eq!(rustia.execution_profile(), expected);
        assert_eq!(rustia.vm_settings().memory_limit_bytes, Some(512 * 1024));
        assert!(!rustia.vm_settings().enable_memory_tracking);

        assert_eq!(
            RustiaBuilder::deterministic().execution_profile(),
            RustiaExecutionProfile::DETERMINISTIC_STANDARD
        );
        assert_ne!(
            RustiaExecutionProfile::DETERMINISTIC_STANDARD.identity_bytes(),
            expected.identity_bytes()
        );

        assert_eq!(
            RustiaBuilder::standard().execution_profile(),
            RustiaBuilder::live().execution_profile(),
            "services and dynamic-code settings are identity inputs owned by the embedder"
        );

        let normalized = Rustia::with_settings(
            Profile::ReproducibleStandard,
            VmSettings {
                guest_surface: GuestSurface::HardenedLeaf,
                memory_limit_bytes: Some(512 * 1024),
                ..VmSettings::default()
            },
        );
        assert_eq!(
            normalized.execution_profile(),
            RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF,
            "live attestation must report the VM-normalized profile, not requested metadata"
        );
    }

    #[test]
    fn deterministic_builder_admits_deterministic_clock_providers() {
        let mut rustia = RustiaBuilder::deterministic()
            .wall_clock(ServiceClock::fixed(1.5))
            .build()
            .unwrap();

        let time: f64 = rustia.run("return os.time()").unwrap();
        assert_eq!(time, 1.5);
        let surface: bool = rustia
            .run(
                "\
return type(os) == 'table'
    and type(os.time) == 'function'
    and type(os.clock) == 'function'
    and type(os.date) == 'function'
    and type(os.difftime) == 'function'
",
            )
            .unwrap();
        assert!(surface);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn deterministic_builder_rejects_system_service_clocks() {
        let result = RustiaBuilder::deterministic()
            .wall_clock(ServiceClock::system())
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject a nondeterministic wall clock");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let result = RustiaBuilder::deterministic()
            .monotonic_clock(ServiceClock::system())
            .build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected deterministic profile to reject a nondeterministic monotonic clock");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn standard_preset_keeps_the_pinned_epoch_and_logical_clock_by_default() {
        let mut rustia = Rustia::builder().build().unwrap();

        let time: f64 = rustia.run("return os.time()").unwrap();
        assert_eq!(time, 1_700_000_000.0);
        let logical: bool = rustia
            .run(
                "\
local first = os.clock()
local second = os.clock()
return first >= 0 and second >= first
",
            )
            .unwrap();
        assert!(logical);
    }

    #[test]
    fn decision_reset_invalidates_every_retained_facade_handle() {
        assert_eq!(RUSTIA_DECISION_RESET_ABI_VERSION, 1);
        let module = Rustia::compile(
            "decision.luau",
            "return { answer = 42, decide = function(x) return x + 1 end }",
        )
        .unwrap();
        let mut rustia = RustiaBuilder::deterministic().build().unwrap();
        let exports = rustia.install(module.clone()).unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();
        let exports_clone = exports.clone();
        let decide_clone = decide.clone();

        assert_eq!(exports.decision_epoch(), 0);
        assert_eq!(decide.decision_epoch(), 0);
        assert_eq!(decide.call::<_, f64>(&mut rustia, (4.0,)).unwrap(), 5.0);

        rustia.reset_decision().unwrap();
        assert_eq!(rustia.decision_epoch(), 1);
        assert!(!exports.is_current(&rustia));
        assert!(!decide.is_current(&rustia));
        for result in [
            exports.get::<f64>(&mut rustia, "answer").map(|_| ()),
            exports_clone.get::<f64>(&mut rustia, "answer").map(|_| ()),
            decide.call::<_, f64>(&mut rustia, ()).map(|_| ()),
            decide_clone.call::<_, f64>(&mut rustia, ()).map(|_| ()),
        ] {
            let error = result.expect_err("pre-reset handle must be stale");
            assert!(
                error.to_string().contains("decision epoch 0"),
                "unexpected stale-handle error: {error}"
            );
        }

        let fresh = rustia.install(module).unwrap();
        let fresh_decide = fresh.function(&mut rustia, "decide").unwrap();
        assert_eq!(fresh.decision_epoch(), 1);
        assert_eq!(
            fresh_decide.call::<_, f64>(&mut rustia, (9.0,)).unwrap(),
            10.0
        );
    }

    #[test]
    fn decision_reset_discards_guest_state_and_restores_host_configuration_and_rng() {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(8).unwrap(),
            enable_memory_tracking: true,
            ..VmSettings::default()
        };
        let extension = RustiaExtension::new("host").fn_pure("bias", || -> f64 { 7.0 });
        let mut rustia = RustiaBuilder::deterministic()
            .settings(settings)
            .wall_clock(ServiceClock::fixed(1_234.0))
            .extension(extension)
            .build()
            .unwrap();
        rustia
            .add_pure_fn("double", |value: f64| value * 2.0)
            .unwrap();
        rustia.insert_global("input", 5.0).unwrap();

        let configured: bool = rustia
            .run(
                "\
decision_global = (decision_global or 0) + 1
return input == 5 and decision_global == 1
    and host.bias() == 7 and double(3) == 6 and os.time() == 1234
",
            )
            .unwrap();
        assert!(configured);
        let first_random: Vec<f64> = rustia
            .run("return { math.random(), math.random(), math.random() }")
            .unwrap();
        assert!(rustia.vm().deterministic_collection_count() > 0);
        let peak_before_reset = rustia.memory_stats().peak_live_bytes.unwrap();

        rustia.reset_decision().unwrap();
        assert_eq!(rustia.vm().deterministic_collection_count(), 0);
        assert_eq!(
            rustia.memory_stats().peak_live_bytes,
            Some(peak_before_reset),
            "lifetime peak telemetry must survive a decision reset"
        );
        let clean_and_configured: bool = rustia
            .run(
                "\
return input == nil and decision_global == nil
    and host.bias() == 7 and double(3) == 6 and os.time() == 1234
",
            )
            .unwrap();
        assert!(clean_and_configured);
        let second_random: Vec<f64> = rustia
            .run("return { math.random(), math.random(), math.random() }")
            .unwrap();
        assert_eq!(first_random, second_random);
        assert!(rustia.vm().deterministic_collection_count() > 0);
    }

    #[test]
    fn decision_reset_reaches_a_many_decision_memory_plateau() {
        let module = Rustia::compile(
            "plateau.luau",
            "\
decision_global = {}
for i = 1, 64 do
    decision_global[i] = { value = i, label = 'temporary-' .. i }
end
local thread = coroutine.create(function()
    coroutine.yield(decision_global)
end)
coroutine.resume(thread)
return { count = #decision_global }
",
        )
        .unwrap();
        let mut rustia = RustiaBuilder::deterministic().build().unwrap();
        let mut plateau = None;

        for decision in 0..600 {
            let exports = rustia.install(module.clone()).unwrap();
            assert_eq!(exports.get::<usize>(&mut rustia, "count").unwrap(), 64);
            rustia.reset_decision().unwrap();
            let stats = rustia.memory_stats();
            if decision == 49 {
                plateau = Some(stats);
            } else if decision > 49 {
                assert_eq!(
                    Some(stats),
                    plateau,
                    "VM memory changed after the warm plateau at decision {decision}"
                );
            }
        }
        assert_eq!(rustia.decision_epoch(), 600);
        let clean: bool = rustia.run("return decision_global == nil").unwrap();
        assert!(clean);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_preset_installs_system_clocks_and_the_stock_loadstring_compiler() {
        let mut rustia = RustiaBuilder::live().build().unwrap();

        let time: f64 = rustia.run("return os.time()").unwrap();
        assert!(
            time >= 1_700_000_000.0,
            "live() os.time() should read the real wall clock, got {time}"
        );
        let clock: f64 = rustia.run("return os.clock()").unwrap();
        assert!(
            clock > 0.0,
            "live() os.clock() should read the real monotonic clock, got {clock}"
        );
        let answer: f64 = rustia.run("return loadstring('return 40 + 2')()").unwrap();
        assert_eq!(answer, 42.0);
        // print goes to real process stdout via `print!`, which a test cannot read back from
        // inside the process; sink delivery is covered by the VM-level capture tests.
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn live_preset_fails_loudly_on_wasm32() {
        let result = RustiaBuilder::live().build();
        let Err(Error::Vm(error)) = result else {
            panic!("expected live() to fail on wasm32");
        };
        assert!(matches!(error.kind, VmErrorKind::Unsupported { .. }));
    }
}
