use crate::function::{host_arg_from_raw, host_return_to_raw, IntoHostFunction, IntoHostReturn};
use crate::value::{
    load_owned_value, snapshot_value, FromRustia, FromRustiaMulti, IntoRustiaMulti,
};
use crate::{
    CompiledModule, Error, Exports, Result, RustiaHostFuelPolicy, RustiaHostFunctionInfo,
    RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
};
use rustia_vm::{
    Error as VmError, ErrorKind as VmErrorKind, HostFunctionOp, OpCapability, Value as RawValue,
    Vm, VmExecution,
};
use std::any::type_name;
use std::convert::Infallible;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The lifetime-independent schema of a typed host boundary.
///
/// A `Rustia<H>` stores functions typed against this schema, while each
/// invocation borrows a fresh [`HostContext<H>`]. Keeping the schema separate
/// lets one VM accept context values containing unrelated tick-local borrows.
pub trait HostSchema: Sized + 'static {
    /// Immutable data supplied afresh for each invocation.
    type Input: ?Sized;
    /// Immutable persistent state visible to slot-read callbacks.
    type Slots: ?Sized;
    /// Mutable state available only to callbacks rejected by
    /// [`Profile::DeterministicSim`](crate::Profile::DeterministicSim).
    type External: ?Sized;
    /// Payload accepted by [`Effects::emit`].
    type Effect;
    /// Payload accepted by [`SlotWrites::write`].
    type SlotWrite;
    /// Host-owned committed record type produced from effects and slot writes.
    type Record;
}

/// The concrete authority borrowed for one invocation of host schema `H`.
///
/// Implementations should treat `input`, `slots`, record construction, and
/// `record_bytes` as pure projections. Rustia can roll back its private
/// journal, but it cannot undo mutation hidden behind interior mutability or
/// mutation performed through [`HostMut`].
pub trait HostContext<H>
where
    H: HostSchema,
{
    /// Returns the immutable input for this invocation.
    fn input(&self) -> &H::Input;

    /// Returns the immutable persistent-slot projection for this invocation.
    fn slots(&self) -> &H::Slots;

    /// Returns mutable external state.
    ///
    /// Rustia calls this only for functions registered through
    /// [`HostModule::external`] or [`HostModule::nondeterministic`], which
    /// deterministic profiles reject before installation.
    fn external_mut(&mut self) -> &mut H::External;

    /// Converts an emitted effect and its dense invocation-local sequence
    /// number into the host's committed record type.
    fn record_effect(&self, sequence: u32, effect: H::Effect) -> H::Record;

    /// Converts a staged slot write and its dense invocation-local sequence
    /// number into the host's committed record type.
    fn record_slot_write(&self, sequence: u32, write: H::SlotWrite) -> H::Record;

    /// Returns the deterministic logical payload size charged to the
    /// invocation output budget.
    fn record_bytes(&self, record: &H::Record) -> u64;
}

impl HostSchema for () {
    type Input = ();
    type Slots = ();
    type External = ();
    type Effect = Infallible;
    type SlotWrite = Infallible;
    type Record = Infallible;
}

impl HostContext<()> for () {
    fn input(&self) -> &() {
        self
    }

    fn slots(&self) -> &() {
        self
    }

    fn external_mut(&mut self) -> &mut () {
        self
    }

    fn record_effect(&self, _sequence: u32, effect: Infallible) -> Infallible {
        match effect {}
    }

    fn record_slot_write(&self, _sequence: u32, write: Infallible) -> Infallible {
        match write {}
    }

    fn record_bytes(&self, record: &Infallible) -> u64 {
        match *record {}
    }
}

/// Monotonic resource limits for one typed invocation.
///
/// Record publication is disabled by default. Enable its independent count
/// and logical-byte budgets explicitly:
///
/// ```
/// use rustia::InvocationLimits;
///
/// let limits = InvocationLimits::new(50_000)
///     .with_max_records(32)
///     .with_max_record_bytes(4 * 1024);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvocationLimits {
    fuel: u64,
    max_records: u32,
    max_record_bytes: u64,
}

impl InvocationLimits {
    /// Creates an invocation budget with `fuel` execution units and record
    /// publication disabled. If the VM also has a configured builder/settings
    /// fuel limit, the lower value is the effective invocation budget.
    pub const fn new(fuel: u64) -> Self {
        Self {
            fuel,
            max_records: 0,
            max_record_bytes: 0,
        }
    }

    /// Sets the maximum number of records that may be staged.
    #[must_use]
    pub const fn with_max_records(mut self, max_records: u32) -> Self {
        self.max_records = max_records;
        self
    }

    /// Sets the maximum total logical bytes of staged records.
    #[must_use]
    pub const fn with_max_record_bytes(mut self, max_record_bytes: u64) -> Self {
        self.max_record_bytes = max_record_bytes;
        self
    }

    /// Returns the shared guest-and-host execution fuel budget.
    pub const fn fuel(self) -> u64 {
        self.fuel
    }

    /// Returns the staged-record count limit.
    pub const fn max_records(self) -> u32 {
        self.max_records
    }

    /// Returns the staged-record logical-byte limit.
    pub const fn max_record_bytes(self) -> u64 {
        self.max_record_bytes
    }
}

/// Monotonic resources consumed by one successful invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvocationUsage {
    pub(crate) fuel_used: u64,
    pub(crate) record_count: u32,
    pub(crate) record_bytes: u64,
}

impl InvocationUsage {
    pub const fn fuel_used(self) -> u64 {
        self.fuel_used
    }

    pub const fn record_count(self) -> u32 {
        self.record_count
    }

    pub const fn record_bytes(self) -> u64 {
        self.record_bytes
    }
}

/// A successful invocation whose ordered records remain in Rustia's reusable
/// journal until the next mutable use of that VM.
#[derive(Debug)]
pub struct InvocationOutcome<'a, T, Record> {
    pub(crate) value: T,
    pub(crate) records: &'a [Record],
    pub(crate) usage: InvocationUsage,
}

impl<'a, T, Record> InvocationOutcome<'a, T, Record> {
    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn records(&self) -> &'a [Record] {
        self.records
    }

    pub const fn usage(&self) -> InvocationUsage {
        self.usage
    }

    pub const fn fuel_used(&self) -> u64 {
        self.usage.fuel_used()
    }

    pub const fn record_count(&self) -> u32 {
        self.usage.record_count()
    }

    pub const fn record_bytes(&self) -> u64 {
        self.usage.record_bytes()
    }

    pub fn into_parts(self) -> (T, &'a [Record], InvocationUsage) {
        (self.value, self.records, self.usage)
    }
}

/// A successful allocation-safe publication into a caller-owned record lane.
#[derive(Debug)]
pub struct InvocationReceipt<T> {
    pub(crate) value: T,
    pub(crate) usage: InvocationUsage,
}

impl<T> InvocationReceipt<T> {
    pub fn value(&self) -> &T {
        &self.value
    }

    pub const fn usage(&self) -> InvocationUsage {
        self.usage
    }

    pub const fn fuel_used(&self) -> u64 {
        self.usage.fuel_used()
    }

    pub const fn record_count(&self) -> u32 {
        self.usage.record_count()
    }

    pub const fn record_bytes(&self) -> u64 {
        self.usage.record_bytes()
    }

    pub fn into_parts(self) -> (T, InvocationUsage) {
        (self.value, self.usage)
    }
}

/// Immutable deterministic input authority.
///
/// The invocation lifetime prevents a borrowed input projection from escaping:
///
/// ```compile_fail
/// use rustia::{HostSchema, Input};
///
/// fn leak<C: HostSchema>(input: Input<'_, C>) -> &'static C::Input {
///     input.get()
/// }
/// ```
#[derive(Clone, Copy)]
pub struct Input<'call, C>
where
    C: HostSchema,
{
    value: &'call C::Input,
}

impl<'call, C> Input<'call, C>
where
    C: HostSchema,
{
    pub fn get(&self) -> &'call C::Input {
        self.value
    }
}

/// Immutable persistent-slot read authority.
#[derive(Clone, Copy)]
pub struct Slots<'call, C>
where
    C: HostSchema,
{
    value: &'call C::Slots,
}

impl<'call, C> Slots<'call, C>
where
    C: HostSchema,
{
    pub fn get(&self) -> &'call C::Slots {
        self.value
    }
}

/// Deterministic effect emission authority.
pub struct Effects<'call, C>
where
    C: HostSchema,
{
    host: &'call dyn HostContext<C>,
    journal: &'call mut EffectJournal<C::Record>,
}

impl<C> Effects<'_, C>
where
    C: HostSchema,
{
    pub fn emit(&mut self, effect: C::Effect) -> Result<()> {
        let sequence = self.journal.next_sequence()?;
        let record = self.host.record_effect(sequence, effect);
        let bytes = self.host.record_bytes(&record);
        self.journal.push(record, bytes)
    }
}

/// Deterministic persistent-slot write authority.
pub struct SlotWrites<'call, C>
where
    C: HostSchema,
{
    host: &'call dyn HostContext<C>,
    journal: &'call mut EffectJournal<C::Record>,
}

impl<C> SlotWrites<'_, C>
where
    C: HostSchema,
{
    pub fn write(&mut self, write: C::SlotWrite) -> Result<()> {
        let sequence = self.journal.next_sequence()?;
        let record = self.host.record_slot_write(sequence, write);
        let bytes = self.host.record_bytes(&record);
        self.journal.push(record, bytes)
    }
}

/// Mutable host authority, available only to external-state and
/// nondeterministic functions rejected by `DeterministicSim`.
pub struct HostMut<'call, C>
where
    C: HostSchema,
{
    value: &'call mut C::External,
}

impl<'call, C> HostMut<'call, C>
where
    C: HostSchema,
{
    pub fn get_mut(&mut self) -> &mut C::External {
        self.value
    }
}

#[derive(Clone, Copy)]
pub(crate) struct JournalCheckpoint {
    len: usize,
    bytes: u64,
}

pub(crate) struct EffectJournal<Record> {
    records: Vec<Record>,
    bytes: u64,
    limits: InvocationLimits,
    hard_error: Option<VmError>,
}

impl<Record> EffectJournal<Record> {
    fn new(records: Vec<Record>, limits: InvocationLimits) -> Self {
        Self {
            records,
            bytes: 0,
            limits,
            hard_error: None,
        }
    }

    fn checkpoint(&self) -> JournalCheckpoint {
        JournalCheckpoint {
            len: self.records.len(),
            bytes: self.bytes,
        }
    }

    fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        self.records.truncate(checkpoint.len);
        self.bytes = checkpoint.bytes;
    }

    fn next_sequence(&mut self) -> Result<u32> {
        if let Some(error) = &self.hard_error {
            return Err(Error::Vm(error.clone()));
        }
        let limit = usize::try_from(self.limits.max_records).unwrap_or(usize::MAX);
        if self.records.len() >= limit {
            return Err(self
                .latch_resource_error("effect-record count", u64::from(self.limits.max_records)));
        }
        u32::try_from(self.records.len())
            .map_err(|_| self.latch_resource_error("effect sequence", u64::from(u32::MAX)))
    }

    fn push(&mut self, record: Record, bytes: u64) -> Result<()> {
        let Some(next_bytes) = self.bytes.checked_add(bytes) else {
            return Err(
                self.latch_resource_error("effect-record bytes", self.limits.max_record_bytes)
            );
        };
        if next_bytes > self.limits.max_record_bytes {
            return Err(
                self.latch_resource_error("effect-record bytes", self.limits.max_record_bytes)
            );
        }
        debug_assert!(self.records.len() < self.records.capacity());
        self.records.push(record);
        self.bytes = next_bytes;
        Ok(())
    }

    fn latch_resource_error(&mut self, resource: &'static str, limit: u64) -> Error {
        let error = VmError::new(VmErrorKind::HostResourceExhausted { resource, limit })
            .without_source_location();
        self.hard_error = Some(error.clone());
        Error::Vm(error)
    }

    fn latch_vm_error(&mut self, error: VmError) {
        if self.hard_error.is_none() {
            self.hard_error = Some(error);
        }
    }

    fn hard_error(&self) -> Option<&VmError> {
        self.hard_error.as_ref()
    }
}

pub(crate) type ContextHandler<C> = Box<
    dyn Fn(
            &mut Vm,
            &mut dyn HostContext<C>,
            &mut EffectJournal<<C as HostSchema>::Record>,
            &[RawValue],
        ) -> rustia_vm::Result<Vec<RawValue>>
        + Send
        + Sync
        + 'static,
>;

pub(crate) struct ContextHandlerEntry<C>
where
    C: HostSchema,
{
    name: String,
    handler: ContextHandler<C>,
}

#[doc(hidden)]
pub struct ContextHostFunction<C>
where
    C: HostSchema,
{
    name: String,
    fuel_cost: u32,
    capability: OpCapability,
    parameter_count: u8,
    parameter_types: Vec<&'static str>,
    return_types: Vec<&'static str>,
    fallible: bool,
    handler: ContextHandler<C>,
}

impl<C> ContextHostFunction<C>
where
    C: HostSchema,
{
    pub(crate) fn into_parts(
        self,
        dispatcher_key: u32,
    ) -> (HostFunctionOp, ContextHandlerEntry<C>) {
        let handler_name = self.name.clone();
        let op =
            HostFunctionOp::dispatched(self.name, self.fuel_cost, self.capability, dispatcher_key)
                .with_signature(
                    self.parameter_count,
                    self.parameter_types,
                    self.return_types,
                    self.fallible,
                );
        (
            op,
            ContextHandlerEntry {
                name: handler_name,
                handler: self.handler,
            },
        )
    }
}

enum HostModuleFunction<C>
where
    C: HostSchema,
{
    Inline(HostFunctionOp),
    Contextual(ContextHostFunction<C>),
}

impl<C> HostModuleFunction<C>
where
    C: HostSchema,
{
    fn name(&self) -> &str {
        match self {
            Self::Inline(function) => &function.name,
            Self::Contextual(function) => &function.name,
        }
    }
}

/// A capability-checked typed host namespace that can be installed as one
/// persistent VM module.
///
/// The views constrain how a callback reaches the invocation context. Safe
/// Rust cannot stop a `'static` callback from using captured globals or
/// interior mutability, so semantic purity remains an embedding contract.
pub struct HostModule<C>
where
    C: HostSchema,
{
    namespace: String,
    functions: Vec<HostModuleFunction<C>>,
}

impl<C> HostModule<C>
where
    C: HostSchema,
{
    /// Starts a typed host module installed under `namespace`.
    ///
    /// The namespace and every function name must be non-empty and unique
    /// within this module; installation validates the complete module before
    /// exposing any binding to guest code.
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            functions: Vec::new(),
        }
    }

    /// Returns the guest-visible namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the versioned policy used to charge host-call fuel.
    pub const fn fuel_policy(&self) -> RustiaHostFuelPolicy {
        RUSTIA_HOST_FUEL_POLICY
    }

    /// Iterates over the module's declared function metadata in install order.
    pub fn functions(&self) -> impl Iterator<Item = RustiaHostFunctionInfo<'_>> + '_ {
        self.functions.iter().map(|function| {
            let (
                name,
                fuel_cost,
                capability,
                parameter_count,
                parameter_types,
                return_types,
                fallible,
            ) = match function {
                HostModuleFunction::Inline(function) => (
                    function.name.as_str(),
                    function.fuel_cost,
                    function.capability,
                    function.parameter_count,
                    function.parameter_types.as_slice(),
                    function.return_types.as_slice(),
                    function.fallible,
                ),
                HostModuleFunction::Contextual(function) => (
                    function.name.as_str(),
                    function.fuel_cost,
                    function.capability,
                    function.parameter_count,
                    function.parameter_types.as_slice(),
                    function.return_types.as_slice(),
                    function.fallible,
                ),
            };
            RustiaHostFunctionInfo {
                name,
                fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                fuel_cost,
                capability,
                parameter_count,
                parameter_types,
                return_types,
                fallible,
            }
        })
    }

    /// Adds a context-free function with [`OpCapability::Pure`].
    ///
    /// `function` has the ordinary shape `Fn(A, B, ...) -> R`; arguments and
    /// returns use Rustia's normal value conversions. A `Result<R>` return is
    /// exposed as a fallible host call.
    pub fn pure<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.functions
            .push(HostModuleFunction::Inline(function.into_host_function(
                name.into(),
                fuel_cost,
                OpCapability::Pure,
            )));
        self
    }

    /// Adds a deterministic input reader.
    ///
    /// The closure shape is `for<'a> Fn(Input<'a, C>, A, B, ...) -> R`.
    /// Its first argument can access only [`HostContext::input`].
    pub fn input<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoInputFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_input_function(name.into(), fuel_cost),
        ));
        self
    }

    /// Adds a deterministic persistent-slot reader.
    ///
    /// The closure shape is `for<'a> Fn(Slots<'a, C>, A, B, ...) -> R`.
    /// Its first argument can access only [`HostContext::slots`].
    pub fn slot_read<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoSlotReadFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_slot_read_function(name.into(), fuel_cost),
        ));
        self
    }

    /// Adds a deterministic effect recorder.
    ///
    /// The closure shape is `for<'a> Fn(Effects<'a, C>, A, B, ...) -> R`.
    /// Calls to [`Effects::emit`] stage ordered records in the current
    /// invocation transaction.
    pub fn effect<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoEffectFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_effect_function(name.into(), fuel_cost),
        ));
        self
    }

    /// Adds a deterministic persistent-slot write recorder.
    ///
    /// The closure shape is
    /// `for<'a> Fn(SlotWrites<'a, C>, A, B, ...) -> R`. Calls to
    /// [`SlotWrites::write`] stage ordered records; they do not mutate slots
    /// directly.
    pub fn slot_write<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoSlotWriteFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_slot_write_function(name.into(), fuel_cost),
        ));
        self
    }

    /// Adds a function that reads or mutates external host state.
    ///
    /// The closure shape is `for<'a> Fn(HostMut<'a, C>, A, B, ...) -> R`.
    /// [`Profile::DeterministicSim`](crate::Profile::DeterministicSim)
    /// rejects the entire module before installation. Mutations through
    /// [`HostMut`] are immediate and cannot be rolled back by Rustia.
    pub fn external<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_external_function(
                name.into(),
                fuel_cost,
                OpCapability::ReadsExternalMutableState,
            ),
        ));
        self
    }

    /// Adds a host operation explicitly classified as nondeterministic.
    ///
    /// It has the same `HostMut` closure shape and rollback caveat as
    /// [`external`](Self::external), but metadata distinguishes an operation
    /// whose effect itself is nondeterministic. Deterministic profiles reject
    /// it before installation.
    pub fn nondeterministic<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        self.functions.push(HostModuleFunction::Contextual(
            function.into_external_function(
                name.into(),
                fuel_cost,
                OpCapability::NondeterministicEffect,
            ),
        ));
        self
    }

    pub(crate) fn install(self, vm: &mut Vm, bindings: &mut HostBindings<C>) -> Result<()> {
        if self.namespace.is_empty() {
            return Err(Error::Embed(
                "typed host module namespace must not be empty".to_owned(),
            ));
        }
        for (index, function) in self.functions.iter().enumerate() {
            if function.name().is_empty() {
                return Err(Error::Embed(
                    "typed host function name must not be empty".to_owned(),
                ));
            }
            if self.functions[..index]
                .iter()
                .any(|prior| prior.name() == function.name())
            {
                return Err(Error::Embed(format!(
                    "duplicate typed host function `{}` in module `{}`",
                    function.name(),
                    self.namespace
                )));
            }
        }
        let contextual_count = self
            .functions
            .iter()
            .filter(|function| matches!(function, HostModuleFunction::Contextual(_)))
            .count();
        bindings
            .handlers
            .try_reserve(contextual_count)
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))?;
        let mut ops = Vec::new();
        ops.try_reserve(self.functions.len())
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))?;
        let mut handlers = Vec::new();
        handlers
            .try_reserve(contextual_count)
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))?;
        for function in self.functions {
            match function {
                HostModuleFunction::Inline(op) => ops.push(op),
                HostModuleFunction::Contextual(function) => {
                    let offset = bindings
                        .handlers
                        .len()
                        .checked_add(handlers.len())
                        .ok_or_else(|| {
                            Error::Vm(VmError::new(VmErrorKind::HostResourceExhausted {
                                resource: "typed host binding count",
                                limit: u64::from(u32::MAX),
                            }))
                        })?;
                    let key = u32::try_from(offset).map_err(|_| {
                        Error::Vm(VmError::new(VmErrorKind::HostResourceExhausted {
                            resource: "typed host binding count",
                            limit: u64::from(u32::MAX),
                        }))
                    })?;
                    let (op, handler) = function.into_parts(key);
                    ops.push(op);
                    handlers.push(handler);
                }
            }
        }
        vm.install_host_module(self.namespace, ops)
            .map_err(Error::Vm)?;
        bindings.handlers.extend(handlers);
        Ok(())
    }
}

pub(crate) struct HostBindings<C>
where
    C: HostSchema,
{
    pub(crate) handlers: Vec<ContextHandlerEntry<C>>,
}

impl<C> Default for HostBindings<C>
where
    C: HostSchema,
{
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }
}

impl<C> HostBindings<C>
where
    C: HostSchema,
{
    pub(crate) fn reserve_one(&mut self) -> Result<()> {
        self.handlers
            .try_reserve(1)
            .map_err(|_| Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))
    }

    pub(crate) fn install(&mut self, vm: &mut Vm, function: ContextHostFunction<C>) -> Result<()> {
        if function.name.is_empty() {
            return Err(Error::Embed(
                "typed host function name must not be empty".to_owned(),
            ));
        }
        self.reserve_one()?;
        let key = u32::try_from(self.handlers.len()).map_err(|_| {
            Error::Vm(VmError::new(VmErrorKind::HostResourceExhausted {
                resource: "typed host binding count",
                limit: u64::from(u32::MAX),
            }))
        })?;
        let (op, handler) = function.into_parts(key);
        vm.install_host_function(op).map_err(Error::Vm)?;
        self.handlers.push(handler);
        Ok(())
    }
}

pub(crate) struct TypedHostDispatcher<'call, C>
where
    C: HostSchema,
{
    handlers: &'call [ContextHandlerEntry<C>],
    context: &'call mut dyn HostContext<C>,
    journal: EffectJournal<C::Record>,
}

impl<'call, C> TypedHostDispatcher<'call, C>
where
    C: HostSchema,
{
    pub(crate) fn new(
        handlers: &'call [ContextHandlerEntry<C>],
        context: &'call mut dyn HostContext<C>,
        records: Vec<C::Record>,
        limits: InvocationLimits,
    ) -> Self {
        Self {
            handlers,
            context,
            journal: EffectJournal::new(records, limits),
        }
    }

    fn invoke(
        &mut self,
        vm: &mut Vm,
        key: u32,
        args: &[RawValue],
    ) -> rustia_vm::Result<Vec<RawValue>> {
        if let Some(error) = self.journal.hard_error() {
            return Err(error.clone());
        }
        let index = usize::try_from(key).map_err(|_| {
            VmError::new(VmErrorKind::HostDispatcherUnavailable { key }).without_source_location()
        })?;
        let entry = self.handlers.get(index).ok_or_else(|| {
            VmError::new(VmErrorKind::HostDispatcherUnavailable { key }).without_source_location()
        })?;
        let checkpoint = self.journal.checkpoint();
        let result = catch_unwind(AssertUnwindSafe(|| {
            (entry.handler)(vm, self.context, &mut self.journal, args)
        }));
        match result {
            Ok(Ok(values)) if self.journal.hard_error().is_none() => Ok(values),
            Ok(result) => {
                self.journal.rollback(checkpoint);
                if let Some(error) = self.journal.hard_error() {
                    Err(error.clone())
                } else {
                    result
                }
            }
            Err(_) => {
                self.journal.rollback(checkpoint);
                let function = entry.name.clone();
                let error =
                    VmError::new(VmErrorKind::HostPanicked { function }).without_source_location();
                self.journal.latch_vm_error(error.clone());
                Err(error)
            }
        }
    }

    fn checkpoint(&self) -> JournalCheckpoint {
        self.journal.checkpoint()
    }

    fn rollback(&mut self, checkpoint: JournalCheckpoint) {
        self.journal.rollback(checkpoint);
    }

    fn latch_if_hard(&mut self, error: &Error) {
        if let Error::Vm(error) = error {
            if error.is_transaction_abort() {
                self.journal.latch_vm_error(error.clone());
            }
        }
    }

    fn hard_error(&self) -> Option<&VmError> {
        self.journal.hard_error()
    }

    pub(crate) fn finish(self) -> (Vec<C::Record>, u64, Option<VmError>) {
        (
            self.journal.records,
            self.journal.bytes,
            self.journal.hard_error,
        )
    }
}

/// A lexically scoped typed host invocation. It cannot be constructed by
/// embedders and exposes no VM re-entry handle to host callbacks.
pub struct Invocation<'call, C>
where
    C: HostSchema,
{
    pub(crate) vm: &'call mut Vm,
    pub(crate) generation: &'call mut u64,
    dispatcher: TypedHostDispatcher<'call, C>,
    execution: VmExecution,
    marker: PhantomData<fn(&mut C)>,
}

impl<'call, C> Invocation<'call, C>
where
    C: HostSchema,
{
    pub(crate) fn new(
        vm: &'call mut Vm,
        generation: &'call mut u64,
        dispatcher: TypedHostDispatcher<'call, C>,
        limits: InvocationLimits,
    ) -> Self {
        let fuel = vm
            .settings()
            .initial_fuel
            .map_or(limits.fuel, |configured| configured.min(limits.fuel));
        let execution = vm.begin_transactional_execution_with_fuel(Some(fuel));
        Self {
            vm,
            generation,
            dispatcher,
            execution,
            marker: PhantomData,
        }
    }

    /// Fuel remaining in the budget shared by every guest call in this
    /// invocation.
    pub const fn fuel_remaining(&self) -> u64 {
        match self.execution.fuel_remaining() {
            Some(fuel) => fuel,
            None => 0,
        }
    }

    pub(crate) fn scoped<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let checkpoint = self.dispatcher.checkpoint();
        let result = operation(self);
        match result {
            Ok(value) if self.dispatcher.hard_error().is_none() => Ok(value),
            Ok(_) => {
                self.dispatcher.rollback(checkpoint);
                Err(Error::Vm(
                    self.dispatcher.hard_error().expect("checked above").clone(),
                ))
            }
            Err(error) => {
                self.dispatcher.rollback(checkpoint);
                self.dispatcher.latch_if_hard(&error);
                if let Some(hard_error) = self.dispatcher.hard_error() {
                    Err(Error::Vm(hard_error.clone()))
                } else {
                    Err(error)
                }
            }
        }
    }

    pub(crate) fn call_retained_raw(
        &mut self,
        function: RawValue,
        args: &[RawValue],
    ) -> Result<Vec<RawValue>> {
        let Self {
            vm,
            dispatcher,
            execution,
            ..
        } = self;
        let mut host = |vm: &mut Vm, key: u32, args: &[RawValue]| dispatcher.invoke(vm, key, args);
        vm.call_retained_function_in(function, args, execution, &mut host)
            .map_err(Error::Vm)
    }

    pub fn install(&mut self, module: CompiledModule) -> Result<Exports> {
        self.scoped(|invocation| {
            let Self {
                vm,
                dispatcher,
                execution,
                generation,
                ..
            } = invocation;
            let mut host =
                |vm: &mut Vm, key: u32, args: &[RawValue]| dispatcher.invoke(vm, key, args);
            let value = vm
                .run_in(&module.program, execution, &mut host)
                .map_err(Error::Vm)?;
            **generation = (**generation).saturating_add(1);
            let value = vm.retain_value(value);
            Ok(Exports::new(value, **generation, vm.decision_epoch()))
        })
    }

    pub fn run<Return>(&mut self, source_name: impl Into<String>, source: &str) -> Result<Return>
    where
        Return: FromRustia,
    {
        let module = crate::Rustia::<C>::compile_module(source_name, source)?;
        self.scoped(|invocation| {
            let exports = invocation.install(module)?;
            let value = exports.value_in(invocation)?;
            Return::from_rustia(value)
        })
    }

    pub(crate) fn convert_call<Args, Return>(
        &mut self,
        function: RawValue,
        args: Args,
    ) -> Result<Return>
    where
        Args: IntoRustiaMulti,
        Return: FromRustiaMulti,
    {
        self.scoped(|invocation| {
            let raw_args = args
                .into_rustia_multi()
                .iter()
                .map(|value| load_owned_value(invocation.vm, value))
                .collect::<Result<Vec<_>>>()?;
            let raw_results = invocation.call_retained_raw(function, &raw_args)?;
            let results = raw_results
                .into_iter()
                .map(|value| snapshot_value(invocation.vm, value))
                .collect::<Result<Vec<_>>>()?;
            Return::from_rustia_multi(results)
        })
    }

    pub(crate) fn finish(self) -> (TypedHostDispatcher<'call, C>, u64) {
        (self.dispatcher, self.execution.fuel_used())
    }
}

mod sealed {
    pub trait Input<C, Args, Return> {}
    pub trait SlotRead<C, Args, Return> {}
    pub trait Effect<C, Args, Return> {}
    pub trait SlotWrite<C, Args, Return> {}
    pub trait External<C, Args, Return> {}
}

#[doc(hidden)]
pub trait IntoInputFunction<C, Args, Return>:
    sealed::Input<C, Args, Return> + Send + Sync + 'static
where
    C: HostSchema,
{
    #[doc(hidden)]
    fn into_input_function(self, name: String, fuel_cost: u32) -> ContextHostFunction<C>;
}

#[doc(hidden)]
pub trait IntoSlotReadFunction<C, Args, Return>:
    sealed::SlotRead<C, Args, Return> + Send + Sync + 'static
where
    C: HostSchema,
{
    #[doc(hidden)]
    fn into_slot_read_function(self, name: String, fuel_cost: u32) -> ContextHostFunction<C>;
}

#[doc(hidden)]
pub trait IntoEffectFunction<C, Args, Return>:
    sealed::Effect<C, Args, Return> + Send + Sync + 'static
where
    C: HostSchema,
{
    #[doc(hidden)]
    fn into_effect_function(self, name: String, fuel_cost: u32) -> ContextHostFunction<C>;
}

#[doc(hidden)]
pub trait IntoSlotWriteFunction<C, Args, Return>:
    sealed::SlotWrite<C, Args, Return> + Send + Sync + 'static
where
    C: HostSchema,
{
    #[doc(hidden)]
    fn into_slot_write_function(self, name: String, fuel_cost: u32) -> ContextHostFunction<C>;
}

#[doc(hidden)]
pub trait IntoExternalFunction<C, Args, Return>:
    sealed::External<C, Args, Return> + Send + Sync + 'static
where
    C: HostSchema,
{
    #[doc(hidden)]
    fn into_external_function(
        self,
        name: String,
        fuel_cost: u32,
        capability: OpCapability,
    ) -> ContextHostFunction<C>;
}

trait MakeCapabilityView<C>
where
    C: HostSchema,
{
    type View<'call>
    where
        C: 'call;

    fn make<'call>(
        context: &'call mut dyn HostContext<C>,
        journal: &'call mut EffectJournal<C::Record>,
    ) -> Self::View<'call>;
}

struct InputView;
struct SlotReadView;
struct EffectView;
struct SlotWriteView;

impl<C> MakeCapabilityView<C> for InputView
where
    C: HostSchema,
{
    type View<'call>
        = Input<'call, C>
    where
        C: 'call;

    fn make<'call>(
        context: &'call mut dyn HostContext<C>,
        _journal: &'call mut EffectJournal<C::Record>,
    ) -> Self::View<'call> {
        Input {
            value: context.input(),
        }
    }
}

impl<C> MakeCapabilityView<C> for SlotReadView
where
    C: HostSchema,
{
    type View<'call>
        = Slots<'call, C>
    where
        C: 'call;

    fn make<'call>(
        context: &'call mut dyn HostContext<C>,
        _journal: &'call mut EffectJournal<C::Record>,
    ) -> Self::View<'call> {
        Slots {
            value: context.slots(),
        }
    }
}

impl<C> MakeCapabilityView<C> for EffectView
where
    C: HostSchema,
{
    type View<'call>
        = Effects<'call, C>
    where
        C: 'call;

    fn make<'call>(
        context: &'call mut dyn HostContext<C>,
        journal: &'call mut EffectJournal<C::Record>,
    ) -> Self::View<'call> {
        Effects {
            host: &*context,
            journal,
        }
    }
}

impl<C> MakeCapabilityView<C> for SlotWriteView
where
    C: HostSchema,
{
    type View<'call>
        = SlotWrites<'call, C>
    where
        C: 'call;

    fn make<'call>(
        context: &'call mut dyn HostContext<C>,
        journal: &'call mut EffectJournal<C::Record>,
    ) -> Self::View<'call> {
        SlotWrites {
            host: &*context,
            journal,
        }
    }
}

macro_rules! context_function {
    (
        $trait_name:ident,
        $sealed_trait:ident,
        $method_name:ident,
        $view:ident,
        $capability:expr,
        $view_factory:ty
    ) => {
        impl<C, Func, Return> sealed::$sealed_trait<C, (), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn($view<'call, C>) -> Return
                + Send
                + Sync
                + 'static,
            Return: IntoHostReturn,
        {
        }

        impl<C, Func, Return> $trait_name<C, (), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn($view<'call, C>) -> Return + Send + Sync + 'static,
            Return: IntoHostReturn,
        {
            fn $method_name(self, name: String, fuel_cost: u32) -> ContextHostFunction<C> {
                let diagnostic_name = name.clone();
                ContextHostFunction {
                    name,
                    fuel_cost,
                    capability: $capability,
                    parameter_count: 0,
                    parameter_types: Vec::new(),
                    return_types: Return::rustia_type_names(),
                    fallible: Return::is_fallible(),
                    handler: Box::new(move |vm, context, journal, _args| {
                        let view =
                            <$view_factory as MakeCapabilityView<C>>::make(context, journal);
                        host_return_to_raw(vm, &diagnostic_name, self(view))
                    }),
                }
            }
        }

        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            1;
            A
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            2;
            A,
            B
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            3;
            A,
            B,
            D
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            4;
            A,
            B,
            D,
            E
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            5;
            A,
            B,
            D,
            E,
            F
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            6;
            A,
            B,
            D,
            E,
            F,
            G
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            7;
            A,
            B,
            D,
            E,
            F,
            G,
            H
        );
        context_function!(
            @arity $trait_name,
            $sealed_trait,
            $method_name,
            $view,
            $capability,
            $view_factory;
            8;
            A,
            B,
            D,
            E,
            F,
            G,
            H,
            I
        );
    };
    (
        @arity $trait_name:ident,
        $sealed_trait:ident,
        $method_name:ident,
        $view:ident,
        $capability:expr,
        $view_factory:ty;
        $parameter_count:expr;
        $($name:ident),+
    ) => {
        impl<C, Func, Return, $($name),+>
            sealed::$sealed_trait<C, ($($name,)+), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn($view<'call, C>, $($name),+) -> Return
                + Send
                + Sync
                + 'static,
            Return: IntoHostReturn,
            $($name: FromRustia,)+
        {
        }

        impl<C, Func, Return, $($name),+> $trait_name<C, ($($name,)+), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn($view<'call, C>, $($name),+) -> Return
                + Send
                + Sync
                + 'static,
            Return: IntoHostReturn,
            $($name: FromRustia,)+
        {
            #[allow(non_snake_case)]
            fn $method_name(self, name: String, fuel_cost: u32) -> ContextHostFunction<C> {
                let diagnostic_name = name.clone();
                ContextHostFunction {
                    name,
                    fuel_cost,
                    capability: $capability,
                    parameter_count: $parameter_count,
                    parameter_types: vec![$(type_name::<$name>()),+],
                    return_types: Return::rustia_type_names(),
                    fallible: Return::is_fallible(),
                    handler: Box::new(move |vm, context, journal, args| {
                        let mut index = 0usize;
                        $(
                            let $name =
                                host_arg_from_raw(vm, args, index, &diagnostic_name)?;
                            index += 1;
                        )+
                        let _ = index;
                        let view =
                            <$view_factory as MakeCapabilityView<C>>::make(context, journal);
                        host_return_to_raw(vm, &diagnostic_name, self(view, $($name),+))
                    }),
                }
            }
        }
    };
}

context_function!(
    IntoInputFunction,
    Input,
    into_input_function,
    Input,
    OpCapability::ReadsDeterministicInput,
    InputView
);
context_function!(
    IntoSlotReadFunction,
    SlotRead,
    into_slot_read_function,
    Slots,
    OpCapability::ReadsSlotState,
    SlotReadView
);
context_function!(
    IntoEffectFunction,
    Effect,
    into_effect_function,
    Effects,
    OpCapability::EmitsDeterministicEffect,
    EffectView
);
context_function!(
    IntoSlotWriteFunction,
    SlotWrite,
    into_slot_write_function,
    SlotWrites,
    OpCapability::WritesSlotEffect,
    SlotWriteView
);

macro_rules! external_function {
    ($parameter_count:expr; $($name:ident),* $(,)?) => {
        impl<C, Func, Return, $($name),*>
            sealed::External<C, ($($name,)*), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn(HostMut<'call, C>, $($name),*) -> Return
                + Send
                + Sync
                + 'static,
            Return: IntoHostReturn,
            $($name: FromRustia,)*
        {
        }

        impl<C, Func, Return, $($name),*> IntoExternalFunction<C, ($($name,)*), Return> for Func
        where
            C: HostSchema,
            Func: for<'call> Fn(HostMut<'call, C>, $($name),*) -> Return
                + Send
                + Sync
                + 'static,
            Return: IntoHostReturn,
            $($name: FromRustia,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn into_external_function(
                self,
                name: String,
                fuel_cost: u32,
                capability: OpCapability,
            ) -> ContextHostFunction<C> {
                let diagnostic_name = name.clone();
                ContextHostFunction {
                    name,
                    fuel_cost,
                    capability,
                    parameter_count: $parameter_count,
                    parameter_types: vec![$(type_name::<$name>()),*],
                    return_types: Return::rustia_type_names(),
                    fallible: Return::is_fallible(),
                    handler: Box::new(move |vm, context, _journal, args| {
                        let mut index = 0usize;
                        $(
                            let $name =
                                host_arg_from_raw(vm, args, index, &diagnostic_name)?;
                            index += 1;
                        )*
                        let _ = index;
                        host_return_to_raw(
                            vm,
                            &diagnostic_name,
                            self(
                                HostMut {
                                    value: context.external_mut(),
                                },
                                $($name),*
                            ),
                        )
                    }),
                }
            }
        }
    };
}

external_function!(0;);
external_function!(1; A);
external_function!(2; A, B);
external_function!(3; A, B, D);
external_function!(4; A, B, D, E);
external_function!(5; A, B, D, E, F);
external_function!(6; A, B, D, E, F, G);
external_function!(7; A, B, D, E, F, G, H);
external_function!(8; A, B, D, E, F, G, H, I);
