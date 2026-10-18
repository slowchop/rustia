use crate::function::{
    host_arg_from_raw, host_return_to_raw, FromHostArgument, IntoHostFunction, IntoHostReturn,
};
use crate::serde_value::from_live;
use crate::value::{
    load_owned_value, snapshot_value, FromRustia, FromRustiaMulti, IntoRustiaMulti,
};
use crate::{
    CompiledEntry, CompiledModule, Error, Exports, Result, RustiaHostFuelPolicy,
    RustiaHostFunctionInfo, Table, Value, RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
};
use rustia_vm::{
    Error as VmError, ErrorKind as VmErrorKind, HostFunctionOp, OpCapability, Value as RawValue,
    Vm, VmExecution, VmHostDispatcher,
};
use std::any::type_name;
use std::convert::Infallible;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Defines the kinds of data Rustia can exchange with your application.
///
/// Implement this trait once for an application integration. It contains no
/// runtime data and has no methods; it only names the Rust types passed between
/// your application and its scripts.
///
/// Here, an *invocation* means one [`Rustia::invoke`](crate::Rustia::invoke)
/// operation: the period in which your application lends data to guest code
/// and collects its outputs.
///
/// A useful way to read the two host traits is:
///
/// - `HostSchema` says **which types** the integration uses.
/// - [`HostContext<H>`](HostContext) supplies **the actual values** for one
///   invocation.
///
/// For example, a game might define its integration like this:
///
/// ```
/// # use rustia::HostSchema;
/// # struct SensorSnapshot;
/// # struct SavedMemory;
/// # struct DebugState;
/// # struct Action;
/// # struct MemoryChange;
/// # struct OutputRecord;
/// struct GameAi;
///
/// impl HostSchema for GameAi {
///     type Input = SensorSnapshot;
///     type Slots = SavedMemory;
///     type External = DebugState;
///     type Effect = Action;
///     type SlotWrite = MemoryChange;
///     type Record = OutputRecord;
/// }
/// ```
///
/// Rust functions exposed to scripts—called *host callbacks*—can request only
/// the part of this schema they are allowed to use. For example, an input
/// callback can read `Input`, while an effect callback can only stage an
/// `Effect`.
///
/// # Why the types and values are separate
///
/// A [`Rustia<H>`](crate::Rustia) VM and its registered functions can live for
/// a long time. The [`HostContext`] passed to [`Rustia::invoke`](crate::Rustia::invoke)
/// is borrowed only while that invocation runs. Keeping them separate means a
/// short-lived context can safely borrow data from the current frame, tick, or
/// request; Rustia never stores those borrowed values.
pub trait HostSchema: Sized + 'static {
    /// Read-only input for the current invocation, such as a sensor snapshot
    /// or request data.
    type Input: ?Sized;
    /// Read-only saved state, or "slots", that persists between invocations.
    type Slots: ?Sized;
    /// Mutable application state for explicitly external or nondeterministic
    /// callbacks.
    ///
    /// [`Profile::DeterministicSim`](crate::Profile::DeterministicSim) rejects
    /// callbacks with this access before installing them.
    type External: ?Sized;
    /// An action or event that a script may stage with [`Effects::emit`].
    type Effect;
    /// A proposed change to saved slot state that a script may stage with
    /// [`SlotWrites::write`].
    type SlotWrite;
    /// One output item returned to the application after a successful
    /// invocation.
    ///
    /// [`HostContext::record_effect`] and [`HostContext::record_slot_write`]
    /// convert effects and slot writes into this common type.
    type Record;
}

/// Supplies the actual host data used by one Rustia invocation.
///
/// This is usually implemented for a small, short-lived struct containing
/// references to your application's current data:
///
/// ```
/// # struct SensorSnapshot;
/// # struct SavedMemory;
/// # struct DebugState;
/// struct CurrentTurn<'a> {
///     sensors: &'a SensorSnapshot,
///     memory: &'a SavedMemory,
///     debug: &'a mut DebugState,
/// }
/// ```
///
/// Pass a value like `CurrentTurn` to [`Rustia::invoke`](crate::Rustia::invoke).
/// Rustia borrows it only until that invocation finishes and never stores it.
/// A later invocation may use a completely different context value, borrowing
/// different application data.
///
/// The accessor methods determine what registered host callbacks can see.
/// Effect and slot-write callbacks do not modify application state directly;
/// they create `Record` values in Rustia's private staging area. Rustia returns
/// those records only if the complete invocation succeeds.
///
/// # What Rustia can roll back
///
/// Rustia can discard staged records, but it cannot undo other mutations.
/// Implementations should therefore make [`HostContext::input`],
/// [`HostContext::slots`], the two `record_*` methods, and
/// [`HostContext::record_bytes`] behave like read-only calculations. Mutation
/// performed through [`HostMut`]—or hidden behind interior mutability—is
/// immediate and is not part of the record transaction.
pub trait HostContext<H>
where
    H: HostSchema,
{
    /// Returns the read-only input for this invocation.
    fn input(&self) -> &H::Input;

    /// Returns the read-only persistent state visible to slot-reading
    /// callbacks.
    fn slots(&self) -> &H::Slots;

    /// Returns mutable application state for external or nondeterministic
    /// callbacks.
    ///
    /// Rustia calls this only for functions registered through
    /// [`HostModule::external`] or [`HostModule::nondeterministic`], which
    /// deterministic profiles reject before installation. Changes made through
    /// this method take effect immediately and cannot be rolled back by Rustia.
    fn external_mut(&mut self) -> &mut H::External;

    /// Wraps an emitted effect in the application's common output-record type.
    ///
    /// `sequence` starts at zero and increases for each effect or slot write
    /// staged during this invocation. It can be used to preserve the exact
    /// order in which the script produced its outputs.
    fn record_effect(&self, sequence: u32, effect: H::Effect) -> H::Record;

    /// Wraps a staged slot write in the application's common output-record
    /// type.
    ///
    /// Effects and slot writes share the same increasing `sequence`, so their
    /// relative order is preserved.
    fn record_slot_write(&self, sequence: u32, write: H::SlotWrite) -> H::Record;

    /// Reports how many logical bytes this record should consume from the
    /// invocation's output budget.
    ///
    /// This is application-defined accounting rather than Rust's in-memory
    /// object size. It must return the same answer for the same record when
    /// deterministic execution is required.
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

/// Fuel and output limits shared by calls within one invocation.
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

/// Resources consumed by one successful invocation.
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

/// The return value, output records, and resource usage of a successful invocation.
///
/// Records borrow Rustia's reusable journal until the next mutable use of that VM.
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

/// The result and usage after an invocation appends its records to your vector.
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

/// Read-only access to an invocation's deterministic input.
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

/// Read-only access to saved application state, or "slots".
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

/// Lets a callback stage actions or events for the application.
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

/// Lets a callback stage changes to saved application state.
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

/// Lets a callback change application state immediately.
///
/// Available only to external-state and nondeterministic callbacks, which
/// `DeterministicSim` rejects. Changes are outside the record transaction.
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
            return Err(Error::from(error.clone()));
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
        Error::from(error)
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
    pub(crate) fn into_op(self) -> HostFunctionOp {
        let handler_name = self.name.clone();
        HostFunctionOp::dispatched(
            self.name,
            self.fuel_cost,
            self.capability,
            std::sync::Arc::new(ContextHandlerEntry::<C> {
                name: handler_name,
                handler: self.handler,
            }),
        )
        .with_signature(
            self.parameter_count,
            self.parameter_types,
            self.return_types,
            self.fallible,
        )
    }
}

/// A group of Rust callbacks installed under one script name.
///
/// Registrations persist across decision resets. Each callback declares which
/// parts of the host context it needs, checked against the selected profile.
///
/// The views constrain how a callback reaches the invocation context. Safe
/// Rust cannot stop a `'static` callback from using captured globals or
/// interior mutability, so semantic purity remains an embedding contract.
/// Callback-panic containment also requires `panic = "unwind"`; an aborting
/// build terminates or traps before Rustia can translate the panic.
pub struct HostModule<C>
where
    C: HostSchema,
{
    namespace: String,
    functions: Vec<HostFunctionOp>,
    schema: PhantomData<fn() -> C>,
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
            schema: PhantomData,
        }
    }

    /// Returns the guest-visible namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Adds the same callback definition accepted by `Rustia::create_function`
    /// and `Rustia::register_function` to this persistent namespace.
    pub fn function<F, S>(mut self, name: impl Into<String>, function: F) -> Self
    where
        F: crate::IntoHostFunctionDefinition<C, S>,
    {
        self.functions
            .push(function.into_definition().into_op(name.into()));
        self
    }

    /// Returns the versioned policy used to charge host-call fuel.
    pub const fn fuel_policy(&self) -> RustiaHostFuelPolicy {
        RUSTIA_HOST_FUEL_POLICY
    }

    /// Iterates over the module's declared function metadata in install order.
    pub fn functions(&self) -> impl Iterator<Item = RustiaHostFunctionInfo<'_>> + '_ {
        self.functions
            .iter()
            .map(|function| RustiaHostFunctionInfo {
                name: &function.name,
                fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                fuel_cost: function.fuel_cost,
                capability: function.capability,
                parameter_count: function.parameter_count,
                parameter_types: &function.parameter_types,
                return_types: &function.return_types,
                fallible: function.fallible,
            })
    }

    /// Adds a context-free function with [`OpCapability::Pure`].
    ///
    /// `function` has the ordinary shape `Fn(A, B, ...) -> R`; arguments and
    /// returns use Rustia's normal value conversions. A `Result<R>` return is
    /// exposed as a fallible host call.
    pub fn pure<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::pure(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a deterministic input reader.
    ///
    /// The closure shape is `for<'a> Fn(Input<'a, C>, A, B, ...) -> R`.
    /// Its first argument can access only [`HostContext::input`].
    pub fn input<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoInputFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::input(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a deterministic persistent-slot reader.
    ///
    /// The closure shape is `for<'a> Fn(Slots<'a, C>, A, B, ...) -> R`.
    /// Its first argument can access only [`HostContext::slots`].
    pub fn slot_read<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoSlotReadFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::slot_read(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a deterministic effect recorder.
    ///
    /// The closure shape is `for<'a> Fn(Effects<'a, C>, A, B, ...) -> R`.
    /// Calls to [`Effects::emit`] stage ordered records in the current
    /// invocation transaction.
    pub fn effect<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoEffectFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::effect(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a deterministic persistent-slot write recorder.
    ///
    /// The closure shape is
    /// `for<'a> Fn(SlotWrites<'a, C>, A, B, ...) -> R`. Calls to
    /// [`SlotWrites::write`] stage ordered records; they do not mutate slots
    /// directly.
    pub fn slot_write<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoSlotWriteFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::slot_write(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a function that reads or mutates external host state.
    ///
    /// The closure shape is `for<'a> Fn(HostMut<'a, C>, A, B, ...) -> R`.
    /// [`Profile::DeterministicSim`](crate::Profile::DeterministicSim)
    /// rejects the entire module before installation. Mutations through
    /// [`HostMut`] are immediate and cannot be rolled back by Rustia.
    pub fn external<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::external(function).fuel_cost(fuel_cost),
        )
    }

    /// Adds a host operation explicitly classified as nondeterministic.
    ///
    /// It has the same `HostMut` closure shape and rollback caveat as
    /// [`external`](Self::external), but metadata distinguishes an operation
    /// whose effect itself is nondeterministic. Deterministic profiles reject
    /// it before installation.
    pub fn nondeterministic<Func, Args, Return>(
        self,
        name: impl Into<String>,
        fuel_cost: u32,
        function: Func,
    ) -> Self
    where
        Func: IntoExternalFunction<C, Args, Return>,
    {
        self.function(
            name,
            crate::HostFunction::nondeterministic(function).fuel_cost(fuel_cost),
        )
    }

    pub(crate) fn install(self, vm: &mut Vm) -> Result<()> {
        if self.namespace.is_empty() {
            return Err(Error::embed(
                "typed host module namespace must not be empty".to_owned(),
            ));
        }
        for (index, function) in self.functions.iter().enumerate() {
            if function.name.is_empty() {
                return Err(Error::embed(
                    "typed host function name must not be empty".to_owned(),
                ));
            }
            if self.functions[..index]
                .iter()
                .any(|prior| prior.name == function.name)
            {
                return Err(Error::embed(format!(
                    "duplicate typed host function `{}` in module `{}`",
                    function.name, self.namespace
                )));
            }
        }
        vm.install_host_module(self.namespace, self.functions)
            .map_err(Error::from)
    }
}

pub(crate) struct TypedHostDispatcher<'call, C>
where
    C: HostSchema,
{
    context: &'call mut dyn HostContext<C>,
    journal: EffectJournal<C::Record>,
    savepoints: Vec<JournalCheckpoint>,
}

impl<'call, C> TypedHostDispatcher<'call, C>
where
    C: HostSchema,
{
    pub(crate) fn new(
        context: &'call mut dyn HostContext<C>,
        records: Vec<C::Record>,
        limits: InvocationLimits,
    ) -> Self {
        Self {
            context,
            journal: EffectJournal::new(records, limits),
            savepoints: Vec::new(),
        }
    }

    fn invoke(
        &mut self,
        vm: &mut Vm,
        handler: &(dyn std::any::Any + Send + Sync),
        args: &[RawValue],
    ) -> rustia_vm::Result<Vec<RawValue>> {
        if let Some(error) = self.journal.hard_error() {
            return Err(error.clone());
        }
        let entry = handler
            .downcast_ref::<ContextHandlerEntry<C>>()
            .ok_or_else(|| {
                VmError::new(VmErrorKind::HostDispatcherUnavailable).without_source_location()
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
        if let Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        } = error
        {
            if error.is_transaction_abort() {
                self.journal.latch_vm_error((**error).clone());
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

impl<C> VmHostDispatcher for TypedHostDispatcher<'_, C>
where
    C: HostSchema,
{
    fn invoke(
        &mut self,
        vm: &mut Vm,
        handler: &(dyn std::any::Any + Send + Sync),
        args: &[RawValue],
    ) -> rustia_vm::Result<Vec<RawValue>> {
        Self::invoke(self, vm, handler, args)
    }

    fn begin_savepoint(&mut self) -> rustia_vm::Result<()> {
        self.savepoints
            .try_reserve(1)
            .map_err(|_| VmError::new(VmErrorKind::OutOfMemory).without_source_location())?;
        self.savepoints.push(self.journal.checkpoint());
        Ok(())
    }

    fn commit_savepoint(&mut self) {
        self.savepoints
            .pop()
            .expect("host journal commit requires an active savepoint");
    }

    fn rollback_savepoint(&mut self) {
        let checkpoint = self
            .savepoints
            .pop()
            .expect("host journal rollback requires an active savepoint");
        self.journal.rollback(checkpoint);
    }
}

/// Runs scripts while borrowing the application's current host context.
///
/// Provided by [`Rustia::invoke`](crate::Rustia::invoke) for the duration of its
/// closure. Host callbacks receive only their allowed context views, without
/// a VM handle that would let them call back into Luau.
pub struct Invocation<'call, C>
where
    C: HostSchema,
{
    pub(crate) vm: &'call mut Vm,
    dispatcher: Option<TypedHostDispatcher<'call, C>>,
    execution: VmExecution,
    installation_transaction_active: bool,
    marker: PhantomData<fn(&mut C)>,
}

impl<'call, C> Invocation<'call, C>
where
    C: HostSchema,
{
    pub(crate) fn new(
        vm: &'call mut Vm,
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
            dispatcher: Some(dispatcher),
            execution,
            installation_transaction_active: false,
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

    /// Creates a borrowed chunk builder using this invocation's shared fuel
    /// budget and typed host dispatcher.
    pub fn load<'chunk, 'source, S>(
        &'chunk mut self,
        source: &'source S,
    ) -> InvocationChunk<'chunk, 'call, 'source, C>
    where
        S: AsRef<[u8]> + ?Sized,
    {
        InvocationChunk::new(self, source.as_ref())
    }

    pub(crate) fn scoped<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let checkpoint = self
            .dispatcher
            .as_ref()
            .expect("invocation dispatcher is available")
            .checkpoint();
        let result = operation(self);
        match result {
            Ok(value)
                if self
                    .dispatcher
                    .as_ref()
                    .expect("invocation dispatcher is available")
                    .hard_error()
                    .is_none() =>
            {
                Ok(value)
            }
            Ok(_) => {
                self.dispatcher
                    .as_mut()
                    .expect("invocation dispatcher is available")
                    .rollback(checkpoint);
                Err(Error::from(
                    self.dispatcher
                        .as_ref()
                        .expect("invocation dispatcher is available")
                        .hard_error()
                        .expect("checked above")
                        .clone(),
                ))
            }
            Err(error) => {
                let dispatcher = self
                    .dispatcher
                    .as_mut()
                    .expect("invocation dispatcher is available");
                dispatcher.rollback(checkpoint);
                dispatcher.latch_if_hard(&error);
                if let Some(hard_error) = dispatcher.hard_error() {
                    Err(Error::from(hard_error.clone()))
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
        let vm = &mut *self.vm;
        let execution = &mut self.execution;
        let dispatcher = self
            .dispatcher
            .as_mut()
            .expect("invocation dispatcher is available");
        vm.call_retained_function_in(function, args, execution, dispatcher)
            .map_err(Error::from)
    }

    pub fn install(&mut self, module: &CompiledModule) -> Result<Exports> {
        self.install_with_environment(module, None)
    }

    fn install_with_environment(
        &mut self,
        module: &CompiledModule,
        environment: Option<RawValue>,
    ) -> Result<Exports> {
        self.scoped(|invocation| {
            let started_outer = !invocation.installation_transaction_active;
            if started_outer {
                invocation
                    .vm
                    .begin_guest_transaction()
                    .map_err(Error::from)?;
                invocation.installation_transaction_active = true;
            }
            if let Err(error) = invocation.vm.begin_guest_transaction() {
                if started_outer {
                    invocation.vm.rollback_guest_transaction();
                    invocation.installation_transaction_active = false;
                }
                return Err(Error::from(error));
            }
            let vm = &mut *invocation.vm;
            let execution = &mut invocation.execution;
            let dispatcher = invocation
                .dispatcher
                .as_mut()
                .expect("invocation dispatcher is available");
            let outcome: Result<Exports> = (|| {
                let module_cohort = vm.create_module_cohort().map_err(Error::from)?;
                let environment = match environment {
                    Some(environment) => environment,
                    None => vm.create_module_environment().map_err(Error::from)?,
                };
                let value = vm
                    .run_installation_in_environment(
                        &module.program,
                        environment,
                        module_cohort,
                        execution,
                        dispatcher,
                    )
                    .map_err(Error::from)?;
                let value = vm.retain_value(value).map_err(Error::from)?;
                Ok(Exports::new(value, vm.decision_epoch()))
            })();
            match outcome {
                Ok(exports) => {
                    if let Err(error) = vm.commit_guest_transaction() {
                        vm.rollback_guest_transaction();
                        if started_outer {
                            vm.rollback_guest_transaction();
                            invocation.installation_transaction_active = false;
                        }
                        Err(Error::from(error).installation_rolled_back())
                    } else {
                        Ok(exports)
                    }
                }
                Err(error) => {
                    vm.rollback_guest_transaction();
                    if started_outer {
                        vm.rollback_guest_transaction();
                        invocation.installation_transaction_active = false;
                    }
                    Err(error.installation_rolled_back())
                }
            }
        })
    }

    /// Executes a precompiled stateless decision body directly.
    ///
    /// Unlike [`install`](Self::install), this does not construct an exports
    /// table, retain bytecode in the VM, or return handles that survive the
    /// invocation. Host effects and slot writes remain transactional under the
    /// surrounding invocation.
    pub fn call_entry<Return>(&mut self, entry: &CompiledEntry) -> Result<Return>
    where
        Return: FromRustia,
    {
        self.scoped(|invocation| {
            let value = {
                let Self {
                    vm,
                    dispatcher,
                    execution,
                    ..
                } = invocation;
                let dispatcher = dispatcher
                    .as_mut()
                    .expect("invocation dispatcher is available");
                vm.run_hardened_leaf_entry_in(&entry.program, execution, dispatcher)
                    .map_err(Error::from)?
            };
            let value = snapshot_value(invocation.vm, value)?;
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
            let owned_args = args.into_rustia_multi()?;
            let raw_args = owned_args
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

    pub(crate) fn finish(
        mut self,
        commit_installation: bool,
    ) -> (TypedHostDispatcher<'call, C>, u64) {
        if self.installation_transaction_active {
            if commit_installation {
                self.vm
                    .commit_guest_transaction()
                    .expect("outer installation transaction has no parent journal");
            } else {
                self.vm.rollback_guest_transaction();
            }
            self.installation_transaction_active = false;
        }
        let dispatcher = self
            .dispatcher
            .take()
            .expect("invocation dispatcher is available");
        (dispatcher, self.execution.fuel_used())
    }

    pub(crate) fn can_commit_installation(&self) -> bool {
        self.dispatcher
            .as_ref()
            .expect("invocation dispatcher is available")
            .hard_error()
            .is_none()
    }
}

/// A borrowed source chunk executed through a typed [`Invocation`].
#[must_use = "an invocation chunk does nothing until eval, eval_live, eval_serde, exec, or install is called"]
pub struct InvocationChunk<'chunk, 'call, 'source, C>
where
    C: HostSchema,
{
    invocation: &'chunk mut Invocation<'call, C>,
    source: &'source [u8],
    source_name: Option<String>,
    source_map: crate::SourceMap,
    environment: Option<Table>,
}

impl<'chunk, 'call, 'source, C> InvocationChunk<'chunk, 'call, 'source, C>
where
    C: HostSchema,
{
    fn new(invocation: &'chunk mut Invocation<'call, C>, source: &'source [u8]) -> Self {
        Self {
            invocation,
            source,
            source_name: None,
            source_map: crate::SourceMap::default(),
            environment: None,
        }
    }

    /// Sets the source name used by compile/runtime diagnostics and as the
    /// referrer passed to the host module resolver for `require` calls.
    pub fn source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    /// Maps a generated console prefix out of human-facing diagnostics.
    pub fn source_map(mut self, mapping: crate::SourceMap) -> Self {
        self.source_map = mapping;
        self
    }

    /// Selects the exact rooted lexical environment.
    pub fn environment(mut self, environment: impl Into<Table>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Evaluates and converts the first result through the owned typed facade.
    pub fn eval<T>(self) -> Result<T>
    where
        T: FromRustia,
    {
        self.execute(|invocation, value| {
            let value = snapshot_value(invocation.vm, value)?;
            T::from_rustia(value)
        })
    }

    /// Evaluates and retains the first result without snapshotting.
    pub fn eval_live(self) -> Result<Value> {
        self.execute(|invocation, value| Value::retain(invocation.vm, value))
    }

    /// Evaluates and deserializes directly from the live VM graph.
    pub fn eval_serde<T>(self) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.execute(|invocation, value| from_live(invocation.vm, value))
    }

    /// Executes the chunk for side effects.
    pub fn exec(self) -> Result<()> {
        self.execute(|_, _| Ok(()))
    }

    /// Atomically installs the chunk. The result remains provisional until
    /// the enclosing typed invocation commits its record transaction.
    pub fn install(self) -> Result<Exports> {
        let (invocation, module, environment) = self.prepare()?;
        let raw_environment = environment
            .as_ref()
            .map(|environment| environment.raw(invocation.vm, "chunk environment handle"))
            .transpose()?;
        invocation.install_with_environment(&module, raw_environment)
    }

    fn execute<T>(
        self,
        convert: impl FnOnce(&mut Invocation<'call, C>, RawValue) -> Result<T>,
    ) -> Result<T> {
        let (invocation, module, environment) = self.prepare()?;
        let raw_environment = environment
            .as_ref()
            .map(|environment| environment.raw(invocation.vm, "chunk environment handle"))
            .transpose()?;
        invocation.scoped(|invocation| {
            let value = {
                let vm = &mut *invocation.vm;
                let execution = &mut invocation.execution;
                let dispatcher = invocation
                    .dispatcher
                    .as_mut()
                    .expect("invocation dispatcher is available");
                match raw_environment {
                    Some(environment) => vm
                        .run_in_environment(&module.program, environment, execution, dispatcher)
                        .map_err(Error::from),
                    None => vm
                        .run_in(&module.program, execution, dispatcher)
                        .map_err(Error::from),
                }?
            };
            convert(invocation, value).map_err(|error| {
                error.with_result_source(
                    module.program.source_name.as_deref().unwrap_or("chunk"),
                    invocation.vm.settings().diagnostic_limits,
                )
            })
        })
    }

    fn prepare(
        self,
    ) -> Result<(
        &'chunk mut Invocation<'call, C>,
        CompiledModule,
        Option<Table>,
    )> {
        let Self {
            invocation,
            source,
            source_name,
            source_map,
            environment,
        } = self;
        if invocation.vm.settings().guest_surface.is_hardened_leaf() {
            return Err(Error::from(
                VmError::new(VmErrorKind::HardenedLeafViolation {
                    feature: "dynamic source loading",
                })
                .without_source_location(),
            ));
        }
        environment
            .as_ref()
            .map(|environment| environment.raw(invocation.vm, "chunk environment handle"))
            .transpose()?;
        source_map.validate(source)?;
        let limits = invocation.vm.settings().diagnostic_limits;
        let mut module = crate::Rustia::compile_module_bytes_with_constructor_order(
            source_name.unwrap_or_else(|| "chunk".to_owned()),
            source,
            invocation.vm.profile() == crate::Profile::Puc51Compat,
            invocation.vm.settings().diagnostic_limits,
        )
        .map_err(|error| {
            source_map
                .apply_error(error, source, limits)
                .admit_diagnostic(invocation.vm)
        })?;
        source_map.apply(&mut module, source, limits);
        Ok((invocation, module, environment))
    }
}

impl<C> std::fmt::Debug for InvocationChunk<'_, '_, '_, C>
where
    C: HostSchema,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InvocationChunk")
            .field("source_bytes", &self.source.len())
            .field("source_name", &self.source_name)
            .field("has_environment", &self.environment.is_some())
            .finish_non_exhaustive()
    }
}

impl<C> Drop for Invocation<'_, C>
where
    C: HostSchema,
{
    fn drop(&mut self) {
        if self.installation_transaction_active {
            self.vm.rollback_guest_transaction();
            self.installation_transaction_active = false;
        }
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
            $($name: FromHostArgument,)+
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
            $($name: FromHostArgument,)+
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
            $($name: FromHostArgument,)*
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
            $($name: FromHostArgument,)*
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
