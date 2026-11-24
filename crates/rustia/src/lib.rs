//! Rustia is a pure-Rust interpreter for standalone Luau. Use it to script a Rust
//! application or replay a simulation on native and WebAssembly targets.
//!
//! Rustia grew out of a game by [Slowchop Studios](https://slowchop.com/) that
//! needed to script character decisions and replay them on desktop and in a
//! browser. Use [`RustiaBuilder::deterministic`] when
//! the same script, seed, and inputs need to produce the same decisions.
//!
//! # A first script
//!
//! Rust gives Luau a score and a function called `double`. Luau calls the function
//! and returns the new score to Rust. In a [new Cargo app], put this in `src/main.rs`.
//!
//! ```rust
//! use rustia::Rustia;
//!
//! fn main() -> rustia::Result<()> {
//!     let mut vm = Rustia::builder().build()?;
//!     vm.insert_global("base_score", 21)?;
//!     vm.add_fn("double", |value: f64| value * 2.0)?;
//!
//!     let score: f64 = vm
//!         .load("return double(base_score)")
//!         .source_name("score.luau")
//!         .eval()?;
//!
//!     println!("The score is {score}");
//!     assert_eq!(score, 42.0);
//!     Ok(())
//! }
//! ```
//!
//! Run `cargo run`. It prints `The score is 42`.
//!
//! # Find what you need
//!
//! | I want to… | Start here |
//! | --- | --- |
//! | Run a script or get a result back | [`Chunk::exec`] runs it. [`Chunk::eval`] returns a Rust value. |
//! | Pass data from Rust to Luau | [`Rustia::insert_global`] sets a named value. [`Rustia::insert_serde`] passes a struct or collection. |
//! | Call a Luau function from Rust | [`Chunk::install`] loads its module. [`Exports::function`] looks up the function for [`Function::call`]. |
//! | Change a script while the app runs | Keep the old functions until the edit installs successfully. See the [hot reload example]. |
//! | Show where a script failed | Give the script a name with [`Chunk::source_name`] and display [`Error::report`]. |
//! | Run Luau in a browser | Follow the [browser setup] or build the [bare WebAssembly example]. |
//!
//! Start with [`Rustia::builder`] for everyday scripting. Use [`RustiaBuilder::live`]
//! when a script needs the current time or a fresh random seed. To limit a script’s
//! work or memory, set [`RustiaBuilder::fuel`] or [`RustiaBuilder::memory_limit_bytes`].
//!
//! # Keeping state across script edits
//!
//! Keep the running app's state in Rust, or save the Luau values you want to
//! carry forward and pass them to the replacement script. The embedder decides
//! which values to keep. A newly created script local starts with its initializer.
//!
//! Use [`Chunk::install`] to try the replacement, then switch to the new exports
//! after it succeeds. If installation fails, Rustia rolls back guest changes and
//! the app can keep using the previous functions. See the [hot reload example].
//!
// Keep comparison rows together in Rustdoc's horizontally scrollable table.
#![doc = "<style>table:has(a[href='#luau-note-1']) :is(td, th) { white-space: nowrap; }</style>"]
#![doc = include_str!("../compare.md")]
//!
//! # Performance
//!
//! The chart compares five workloads on Linux x86-64. Each language has its own
//! version. Roto's array workload uses a handwritten quadratic sort. It uses a
//! list for sparse lookups and an ordinary function for the closure test.
//! Compare times within a column. Lower is better.
//! The upstream C++ Luau interpreter ran all five faster than Rustia in this run.
//!
#![doc = include_str!("../performance.svg")]
//!
//! These October 2026 measurements cover scripts already running. Small timing
//! differences may vary between runs. [Full results] · [Benchmark limitations]
//!
//! # More about embedding
//!
//! <details>
//! <summary>Read a Luau table from Rust</summary>
//!
//! Use [`Chunk::eval`] to copy a value into Rust, or [`Chunk::eval_serde`] to read a
//! table into a Rust struct. These copies stay usable after the interpreter is gone.
//! Copying strings into a Rust `String` requires UTF-8. Tables with cycles cannot
//! be copied, and copying the same table twice makes two independent values.
//!
//! Use [`Chunk::eval_live`] to keep a table with shared references, cycles, or
//! binary strings in Luau. A live [`Value`], [`Table`], [`Function`], or [`Exports`]
//! belongs to one interpreter. Resetting it with [`Rustia::reset_decision`] invalidates
//! those handles. See the [live values example] for a table that refers to itself.
//!
//! </details>
//!
//! <details>
//! <summary>Give scripts access to Rust functions and objects</summary>
//!
//! Use [`Rustia::add_fn`] for a calculation such as `double`. Rustia trusts that
//! the function only uses its inputs. In the standard and live environments,
//! scripts can change a Rust object immediately. See the [mutable object example].
//!
//! If a callback needs data borrowed for just one request or frame, pass it in a
//! [`HostContext`] with [`Rustia::invoke`]. The [typed invocation example] shows how.
//! For deterministic replay, let scripts propose changes for Rust to apply
//! after a successful call. See the [game object example]. [`HostFunction`] documents callback permissions
//! and the rules for data captured by a closure.
//!
//! </details>
//!
//! <details>
//! <summary>Keep using the interpreter after a script fails</summary>
//!
//! | What you ran | What happens when an error reaches Rust |
//! | --- | --- |
//! | A script or function with [`Chunk::eval`], [`Chunk::exec`], or [`Function::call`] | Changes the script already made stay in the interpreter, even if converting the result to Rust failed. |
//! | A replacement module with [`Chunk::install`] | Rustia restores the previous script state. You can keep calling the old functions. |
//! | A call with [`Rustia::invoke`] or [`Rustia::invoke_into`] | Rust receives none of the changes the script queued for it. Changes made in Luau still survive, except those rolled back by [`Invocation::install`]. |
//! | A stateless decision with [`Rustia::invoke_entry_into`] | Rustia discards the queued changes and resets the script state. It also resets after success. Rust can reuse the [`CompiledEntry`] for the next call. See the [stateless decision example]. |
//!
//! A Luau `pcall` can catch a script error. If the outer invocation succeeds, it
//! can still deliver changes queued by earlier successful callbacks. Resetting
//! the interpreter clears its script state and releases the Rust objects it holds.
//! Files a callback wrote and changes it made to your application’s state persist.
//!
//! An ordinary script error lets you keep using the interpreter. If Rustia catches
//! a panic in a Rust callback, call [`Rustia::reset_decision`] before running more
//! Luau. For exhausted limits, [`VmError::kind`] identifies the resource.
//!
//! </details>
//!
//! For more, [browse the Rust examples] or read about [moving an idle interpreter
//! to another thread](struct.Rustia.html#threads).
//!
//! [new Cargo app]: https://github.com/slowchop/rustia#try-it
//! [hot reload example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs
//! [browser setup]: https://github.com/slowchop/rustia/blob/main/demos/web-live/README.md
//! [bare WebAssembly example]: https://github.com/slowchop/rustia/blob/main/demos/live-wasm/README.md
//! [full results]: https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/RESULTS.md
//! [benchmark limitations]: https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/CLAIMS.md
//! [live values example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/live_values.rs
//! [mutable object example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/mutable_objects.rs
//! [typed invocation example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/typed_invocation.rs
//! [game object example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/game_objects.rs
//! [stateless decision example]: https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hardened_decision.rs
//! [browse the Rust examples]: https://github.com/slowchop/rustia/tree/main/crates/rustia/examples

#[cfg(doctest)]
mod brand_contract;
mod chunk;
mod diagnostic;
mod diagnostic_data;
#[cfg(test)]
mod engine_boundary_tests;
mod error;
mod extension_api;
mod function;
mod host_function;
mod invocation;
mod live_value;
mod macros;
mod object;
mod object_context;
mod object_conversion;
mod object_method;
mod profile;
mod runtime_error;
mod serde_value;
mod settings;
mod source_error;
mod source_map;
mod value;

use invocation::TypedHostDispatcher;
use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_vm::{HardenedLeafProgram, Program, Vm};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

#[cfg(test)]
extern crate self as rustia;

pub use chunk::Chunk;
pub use diagnostic::DiagnosticReason;
pub use diagnostic::{
    Diagnostic, DiagnosticFrame, DiagnosticLimits, DiagnosticSpan, ErrorReport, SourceSnapshot,
};
pub use error::{Error, ErrorKind};
pub use extension_api::{
    ModuleSource, RustiaBuilder, RustiaExecutionProfile, RustiaExtension, RustiaHostFuelPolicy,
    RustiaHostFunctionInfo, ServiceClock, ServiceEntropy, ServiceModuleResolver,
    RUSTIA_DEFAULT_HOST_FUNCTION_FUEL, RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
    RUSTIA_MIN_HOST_FUNCTION_FUEL,
};
pub use function::{
    Exports, FromHostArgument, Function, IntoHostFunction, IntoHostReturn, IntoHostValue,
};
pub use host_function::{HostFunction, IntoHostFunctionDefinition};
pub use invocation::{
    Effects, HostContext, HostModule, HostMut, HostSchema, Input, IntoEffectFunction,
    IntoExternalFunction, IntoInputFunction, IntoSlotReadFunction, IntoSlotWriteFunction,
    Invocation, InvocationChunk, InvocationLimits, InvocationOutcome, InvocationReceipt,
    InvocationUsage, SlotWrites, Slots,
};
pub use live_value::{Table, Value, ValueType};
pub use object::{NewObject, Object, ObjectType};
pub use object_context::{
    IntoEffectObjectMethod, IntoExternalMutObjectMethod, IntoExternalObjectMethod,
    IntoInputObjectMethod, IntoSlotReadObjectMethod, IntoSlotWriteObjectMethod,
};
pub use object_method::{IntoMutObjectMethod, IntoObjectIterator, IntoObjectMethod};
pub use profile::{GuestSurface, OpCapability, Profile};
pub use runtime_error::{VmError, VmErrorKind, VmErrorMessage};
/// Version of the decision-reset lifecycle contract.
pub const RUSTIA_DECISION_RESET_ABI_VERSION: u32 = rustia_vm::DECISION_RESET_ABI_VERSION;
/// Version of the hardened-leaf guest environment contract.
pub const RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION: u32 = rustia_vm::HARDENED_LEAF_GUEST_ABI_VERSION;
/// Version of installation rollback and publication behavior.
pub const RUSTIA_INSTALL_TRANSACTION_ABI_VERSION: u32 = rustia_vm::INSTALL_TRANSACTION_ABI_VERSION;
/// Version of module resolution, suspension, and cache behavior.
///
/// Replay protocols should record this value and their resolver's own content
/// identity. See [`ServiceModuleResolver`] for loading and failure rules.
pub const RUSTIA_MODULE_RESOLUTION_ABI_VERSION: u32 = rustia_vm::MODULE_RESOLUTION_ABI_VERSION;
pub use serde_value::SerdeValueError;
pub use settings::{ServiceDeterminism, Settings, VmMemoryStats};
pub use source_error::{CompileError, LexError, ParseError, SourceError, UnclosedBlock};
pub use source_map::SourceMap;
pub use value::{
    FromRustia, FromRustiaMulti, IntoRustia, IntoRustiaMulti, OwnedKey, OwnedTable, OwnedValue,
};

/// Owns one Luau interpreter and the Rust functions installed in it.
///
/// Create one with [`builder`](Self::builder), then use [`load`](Self::load) to
/// run scripts. Globals and script objects persist between calls until
/// [`reset_decision`](Self::reset_decision); an ordinary evaluation error does
/// not undo earlier script mutations.
///
/// ```
/// use rustia::Rustia;
///
/// let mut vm = Rustia::builder().build()?;
/// vm.insert_global("name", "Ada")?;
/// let greeting: String = vm.eval("return 'Hello, ' .. name")?;
/// assert_eq!(greeting, "Hello, Ada");
/// # Ok::<(), rustia::Error>(())
/// ```
///
/// # Threads
///
/// By default, an interpreter and its live handles are thread-affine. Create
/// each interpreter in the worker that will run it. Enabling the Cargo feature
/// `transferable` makes the interpreter and its live handles `Send` when their
/// typed host record and Rust payload types are also transferable. Move the
/// whole idle interpreter and its handles together; every execution operation
/// still needs exclusive `&mut` access, so this does not permit concurrent entry.
/// Transferable service closures require `Send`, entropy providers require
/// `Fn + Send + Sync`, and object payloads require `Send + Sync`. Host-owned
/// [`CompiledModule`] and [`CompiledEntry`] values can
/// always be reused across interpreters without sharing their guest state.
///
/// # Typed host contexts
///
/// The default `Rustia<()>` needs no per-call host context. For applications that
/// lend state to a script for one call, `Rustia<H>` stores a lifetime-independent
/// [`HostSchema`] and registered callbacks. Each execution borrows a fresh
/// [`HostContext<H>`] through
/// [`invoke`](Self::invoke) or [`invoke_into`](Self::invoke_into).
///
/// <details>
/// <summary>Why typed hosts require an invocation</summary>
///
/// Typed hosts cannot bypass that authority boundary with the context-free
/// execution helpers:
///
/// ```compile_fail
/// use rustia::{HostSchema, Rustia};
///
/// struct Host;
/// impl HostSchema for Host {
///     type Input = ();
///     type Slots = ();
///     type External = ();
///     type Effect = ();
///     type SlotWrite = ();
///     type Record = ();
/// }
///
/// fn bypass(mut rustia: Rustia<Host>) {
///     let _: i32 = rustia.eval("return 42").unwrap();
/// }
/// ```
///
/// </details>
pub struct Rustia<C: HostSchema = ()> {
    vm: Vm,
    journal: Vec<C::Record>,
}

impl<C> Rustia<C>
where
    C: HostSchema,
{
    #[cfg(test)]
    fn from_profile(profile: Profile) -> Self {
        Self {
            vm: Vm::new(profile.to_core()),
            journal: Vec::new(),
        }
    }

    fn from_settings(profile: Profile, settings: rustia_vm::VmSettings) -> Self {
        Self {
            vm: Vm::with_settings(profile.to_core(), settings),
            journal: Vec::new(),
        }
    }

    /// Reports live and reserved memory owned by the interpreter.
    ///
    /// These are logical accounting totals, not process memory usage. Captured
    /// Rust callback state is outside these totals; diagnostic source retention
    /// has a separate budget. See [`VmMemoryStats`] for the totals and optional
    /// instruction-boundary peaks.
    pub fn memory_stats(&mut self) -> VmMemoryStats {
        VmMemoryStats::from_core(self.vm.memory_stats())
    }

    /// Returns the bytes charged for retained diagnostic source snapshots.
    ///
    /// This host-side retention is separate from the script memory limit.
    pub fn diagnostic_source_bytes(&self) -> usize {
        self.vm.diagnostic_source_bytes()
    }

    /// Returns the selected language profile and guest surface.
    ///
    /// This descriptor does not include budgets, services, or host callbacks.
    pub fn execution_profile(&self) -> RustiaExecutionProfile {
        RustiaExecutionProfile::from_vm(self.vm.profile(), self.vm.settings().guest_surface)
    }

    /// Returns the interpreter's effective application settings.
    ///
    /// Use [`Settings::identity_bytes`] when binding configuration to a replay
    /// or package identity. Engine implementation controls remain private.
    pub fn settings(&self) -> Settings {
        Settings::from_core(Profile::from_core(self.vm.profile()), self.vm.settings())
    }

    #[cfg(test)]
    pub(crate) const fn vm_settings(&self) -> &rustia_vm::VmSettings {
        self.vm.settings()
    }

    /// Returns the generation used to validate live handles.
    ///
    /// It starts at zero and advances when [`reset_decision`](Self::reset_decision)
    /// succeeds.
    pub fn decision_epoch(&self) -> u64 {
        self.vm.decision_epoch()
    }

    /// Discards all guest state and begins a fresh decision epoch.
    ///
    /// VM configuration, services, extensions, and installed host functions
    /// persist. Globals inserted with [`insert_global`](Self::insert_global)
    /// and [`insert_serde`](Self::insert_serde) are decision inputs and must be
    /// rebound afterward. Every prior [`Value`], [`Table`], [`Function`], and
    /// [`Exports`] handle is invalidated, while host-owned [`CompiledModule`]
    /// and [`CompiledEntry`] values remain reusable. A hardened-leaf VM preserves its sealed
    /// immutable environment in place and discards every decision-local
    /// allocation after that seal.
    ///
    /// Replay protocols must record
    /// [`RUSTIA_DECISION_RESET_ABI_VERSION`].
    /// Reset requires an idle interpreter; attempting it during execution fails.
    pub fn reset_decision(&mut self) -> Result<()> {
        self.vm.reset_decision().map_err(Error::from_engine)?;
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
            Error::from(VmError::new(VmErrorKind::HostResourceExhausted {
                resource: "effect-record count",
                limit: u64::from(limits.max_records()),
            }))
        })?;
        records
            .try_reserve(reserve)
            .map_err(|_| Error::from(VmError::new(VmErrorKind::OutOfMemory)))?;
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
            Error::from(VmError::new(VmErrorKind::HostResourceExhausted {
                resource: "effect-record count",
                limit: u64::from(limits.max_records()),
            }))
        })?;
        self.journal
            .try_reserve(reserve)
            .map_err(|_| Error::from(VmError::new(VmErrorKind::OutOfMemory)))?;

        let journal = std::mem::take(&mut self.journal);
        let dispatcher = TypedHostDispatcher::new(context, journal, limits);
        let mut invocation = Invocation::new(&mut self.vm, dispatcher, limits);
        let result = operation(&mut invocation);
        let commit_installation = result.is_ok() && invocation.can_commit_installation();
        let (dispatcher, fuel_used) = invocation.finish(commit_installation);
        let (journal, record_bytes, hard_error) = dispatcher.finish();
        self.journal = journal;
        if let Some(error) = hard_error {
            self.journal.clear();
            return Err(Error::from_engine(error));
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
            Err(Error::from(VmError::new(VmErrorKind::VmPoisoned)))
        } else {
            Ok(())
        }
    }

    /// Copies Rust data into a script global, replacing any existing value.
    ///
    /// Uses [`IntoRustia`] for conversion. The value is available to later scripts
    /// until replaced or [`reset_decision`](Self::reset_decision) clears it.
    /// Conversion errors and script memory-limit failures are returned as [`Error`].
    /// To insert a live handle without copying it, use [`Table::set_live`] on
    /// [`globals`](Self::globals).
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// vm.insert_global("scores", vec![10, 20, 30])?;
    /// let first: i32 = vm.eval("return scores[1]")?;
    /// assert_eq!(first, 10);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn insert_global<T>(&mut self, name: impl AsRef<str>, value: T) -> Result<()>
    where
        T: IntoRustia,
    {
        let value = value.into_rustia()?;
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm
            .set_global_value(name, value)
            .map_err(Error::from_engine)
    }

    /// Serializes Rust data into a script global, replacing any existing value.
    ///
    /// This copies the data; later script changes do not modify the Rust input.
    /// Serialization and script memory-limit failures are returned as [`Error`].
    /// Like [`insert_global`](Self::insert_global), the binding is cleared by reset.
    pub fn insert_serde<T>(&mut self, name: impl AsRef<str>, value: &T) -> Result<()>
    where
        T: serde::Serialize + ?Sized,
    {
        let value = OwnedValue::from_serialize(value)?;
        let value = value::load_owned_value(&mut self.vm, &value)?;
        self.vm
            .set_global_value(name, value)
            .map_err(Error::from_engine)
    }

    /// Returns a live handle to the interpreter's globals table.
    ///
    /// Reads and writes through [`Table`] preserve identity and are visible to
    /// subsequently executed guest code. Like ordinary global inputs, these
    /// mutations are decision-local and are discarded by
    /// [`reset_decision`](Self::reset_decision). Hardened leaves reject this
    /// mutable authority.
    pub fn globals(&mut self) -> Result<Table> {
        let value = self.vm.retain_globals().map_err(Error::from_engine)?;
        Ok(Table::new(Value::new(value, self.vm.decision_epoch())))
    }

    /// Creates an empty live table in this interpreter.
    ///
    /// The table and its root participate in the logical-live memory quota as
    /// one admission. Hardened leaves reject live table construction.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// let settings = vm.create_table()?;
    /// settings.set(&mut vm, "volume", 0.5)?;
    /// vm.globals()?.set_live(&mut vm, "settings", &settings.as_value())?;
    /// let volume: f64 = vm.eval("return settings.volume")?;
    /// assert_eq!(volume, 0.5);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn create_table(&mut self) -> Result<Table> {
        let value = self
            .vm
            .create_retained_table()
            .map_err(Error::from_engine)?;
        Ok(Table::new(Value::new(value, self.vm.decision_epoch())))
    }

    /// Creates an immutable live Luau vector in this interpreter.
    ///
    /// The vector and its root participate in the logical-live memory quota as
    /// one admission. Components use Luau's canonical `f32` vector semantics.
    /// The returned handle belongs to this VM and decision epoch like every
    /// other live [`Value`].
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// let direction = vm.create_vector(1.5, -2.0, 3.0)?;
    /// vm.globals()?.set_live(&mut vm, "direction", &direction)?;
    /// let x: f64 = vm.eval("return direction.x")?;
    /// assert_eq!(x, 1.5);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn create_vector(&mut self, x: f32, y: f32, z: f32) -> Result<Value> {
        let value = self
            .vm
            .create_retained_vector(x, y, z)
            .map_err(Error::from_engine)?;
        Ok(Value::new(value, self.vm.decision_epoch()))
    }

    fn ensure_dynamic_source_loading_allowed(&self) -> Result<()> {
        if self.vm.settings().guest_surface.is_hardened_leaf() {
            return Err(Error::from(
                VmError::new(VmErrorKind::HardenedLeafViolation {
                    feature: "dynamic source loading",
                })
                .without_source_location(),
            ));
        }
        self.ensure_healthy()
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
        self.register_function(name, HostFunction::pure(function).fuel_cost(fuel_cost))
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
        self.register_function(name, HostFunction::input(function).fuel_cost(fuel_cost))
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
        self.register_function(name, HostFunction::slot_read(function).fuel_cost(fuel_cost))
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
        self.register_function(name, HostFunction::effect(function).fuel_cost(fuel_cost))
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
        self.register_function(
            name,
            HostFunction::slot_write(function).fuel_cost(fuel_cost),
        )
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
        self.register_function(name, HostFunction::external(function).fuel_cost(fuel_cost))
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
        self.register_function(
            name,
            HostFunction::nondeterministic(function).fuel_cost(fuel_cost),
        )
    }

    /// Installs a namespaced collection of capability-checked host functions.
    ///
    /// Installation is fail-closed: if validation or VM binding fails, no
    /// callable namespace or typed dispatcher bindings are retained.
    pub fn install_host_module(&mut self, module: HostModule<C>) -> Result<()> {
        module.install(&mut self.vm)
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
    /// Compiles a script once for later installation in one or more interpreters.
    ///
    /// Compilation does not run the script or change a VM. `source_name` labels
    /// diagnostics and identifies the caller to the module resolver for `require`.
    /// Invalid source returns a parse or compilation error. Call
    /// [`install`](Self::install) to run the compiled module and retain its exports.
    /// For PUC Lua 5.1 compatibility, use
    /// [`compile_module_with_profile`](Self::compile_module_with_profile).
    pub fn compile_module(source_name: impl Into<String>, source: &str) -> Result<CompiledModule> {
        Self::compile_module_bytes(source_name, source.as_bytes())
    }

    /// Compiles a reusable module with code generation appropriate for
    /// `profile`. Use this when the module will be installed into a VM whose
    /// profile is known ahead of time. `source_name` labels diagnostics and
    /// nested `require` resolver referrers.
    pub fn compile_module_with_profile(
        source_name: impl Into<String>,
        source: &str,
        profile: Profile,
    ) -> Result<CompiledModule> {
        Self::compile_module_bytes_with_profile(source_name, source.as_bytes(), profile)
    }

    /// Compiles bytes with the standard (source-order) table-constructor write
    /// order. `source_name` labels diagnostics and nested `require` resolver
    /// referrers. `Profile::Puc51Compat` hosts need PUC write order instead;
    /// compile through the instance `load` builder (which reads the VM profile)
    /// or use [`compile_module_bytes_with_profile`](Self::compile_module_bytes_with_profile).
    pub fn compile_module_bytes(
        source_name: impl Into<String>,
        source: &[u8],
    ) -> Result<CompiledModule> {
        Self::compile_module_bytes_with_constructor_order(
            source_name,
            source,
            false,
            DiagnosticLimits::default(),
        )
    }

    /// Byte-oriented form of [`compile_module_with_profile`](Self::compile_module_with_profile).
    pub fn compile_module_bytes_with_profile(
        source_name: impl Into<String>,
        source: &[u8],
        profile: Profile,
    ) -> Result<CompiledModule> {
        Self::compile_module_bytes_with_constructor_order(
            source_name,
            source,
            profile == Profile::Puc51Compat,
            DiagnosticLimits::default(),
        )
    }

    fn compile_module_bytes_with_constructor_order(
        source_name: impl Into<String>,
        source: &[u8],
        puc51_constructor_order: bool,
        diagnostic_limits: DiagnosticLimits,
    ) -> Result<CompiledModule> {
        let source_name = source_name.into();
        let chunk = rustia_parser::parse_source_bytes(source).map_err(|error| {
            Error::from_engine(error).with_source(&source_name, source, diagnostic_limits)
        })?;
        let options = CompileOptions {
            puc51_constructor_order,
            ..CompileOptions::default()
        }
        .with_source_bytes(source_name.clone(), source);
        let mut program = compile_with_options(&chunk, options).map_err(|error| {
            Error::from_engine(error).with_source(&source_name, source, diagnostic_limits)
        })?;
        program.diagnostic_source = rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
            source,
            diagnostic_limits.source_bytes,
        );
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
    ///
    /// Compiles with the standard (source-order) table-constructor write
    /// order; see [`compile_module_bytes`](Self::compile_module_bytes).
    pub fn compile_entry(source_name: impl Into<String>, source: &str) -> Result<CompiledEntry> {
        let source_name = source_name.into();
        let chunk = rustia_parser::parse_source(source).map_err(Error::from_engine)?;
        let options = CompileOptions::default().with_source(source_name.clone(), source);
        let program = compile_with_options(&chunk, options).map_err(Error::from_engine)?;
        let program = HardenedLeafProgram::new(program).map_err(Error::from_engine)?;
        Ok(CompiledEntry {
            source_name,
            program,
        })
    }

    /// Returns the [`standard`](RustiaBuilder::standard) preset:
    /// reproducible-by-default, with a real stdout `print` sink on native
    /// targets.
    pub fn builder() -> RustiaBuilder {
        RustiaBuilder::standard()
    }

    /// Creates a service-free interpreter with the selected language profile.
    ///
    /// Unlike [`builder`](Self::builder), this constructor installs no stdout
    /// sink. Use the builder presets for normal application defaults and host
    /// clock, output, entropy, or module services.
    #[cfg(test)]
    pub(crate) fn new(profile: Profile) -> Self {
        Self::from_profile(profile)
    }

    /// Creates an interpreter with explicit low-level settings and services.
    ///
    /// Prefer [`builder`](Self::builder) for ordinary application configuration.
    #[cfg(test)]
    pub(crate) fn with_settings(profile: Profile, settings: rustia_vm::VmSettings) -> Self {
        Self::from_settings(profile, settings)
    }

    /// Creates a typed interpreter with explicit low-level settings and services.
    ///
    /// Prefer [`RustiaBuilder::build_for_host`] for application configuration.
    #[cfg(test)]
    pub(crate) fn for_host_with_settings<H>(
        profile: Profile,
        settings: rustia_vm::VmSettings,
    ) -> Rustia<H>
    where
        H: HostSchema,
    {
        Rustia::<H>::from_settings(profile, settings)
    }

    /// Prepares Luau source text or bytes to run in this interpreter.
    ///
    /// Building the chunk is inert: parsing, compilation, VM allocation, and
    /// execution begin only when a terminal operation is called.
    /// Use [`Chunk::source_name`] to label errors, then choose [`Chunk::eval`],
    /// [`Chunk::exec`], or [`Chunk::install`]. The source only needs to live until
    /// that operation finishes; returned values do not borrow it.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// let answer: i32 = vm
    ///     .load("return 6 * 7")
    ///     .source_name("answer.luau")
    ///     .eval()?;
    /// assert_eq!(answer, 42);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn load<'rustia, 'source, S>(
        &'rustia mut self,
        source: &'source S,
    ) -> Chunk<'rustia, 'source>
    where
        S: AsRef<[u8]> + ?Sized,
    {
        Chunk::new(self, source.as_ref())
    }

    /// Evaluates source and converts its first result through [`FromRustia`].
    ///
    /// Shorthand for `load(source).eval()`. Use [`load`](Self::load) to give the
    /// source a name or select its environment. See [`Chunk::eval`] for conversion
    /// and failure behavior.
    pub fn eval<T>(&mut self, source: impl AsRef<[u8]>) -> Result<T>
    where
        T: FromRustia,
    {
        let source = source;
        self.load(&source).eval()
    }

    /// Evaluates source and deserializes its first result directly from the VM.
    ///
    /// Shorthand for `load(source).eval_serde()`; see [`Chunk::eval_serde`].
    pub fn eval_serde<T>(&mut self, source: impl AsRef<[u8]>) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let source = source;
        self.load(&source).eval_serde()
    }

    /// Executes source for side effects and discards its result.
    ///
    /// Shorthand for `load(source).exec()`; see [`Chunk::exec`].
    pub fn exec(&mut self, source: impl AsRef<[u8]>) -> Result<()> {
        let source = source;
        self.load(&source).exec()
    }

    /// Runs a compiled module in a fresh environment and retains its exports.
    ///
    /// The environment inherits VM globals. A failed installation restores prior
    /// guest state; external host callback and module-resolver effects cannot be
    /// undone. Previously installed exports remain usable until reset.
    /// See [`Chunk::install`] to compile and install source in one step.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// let module = Rustia::compile_module(
    ///     "greeting.luau",
    ///     "return { greet = function(name) return 'Hello, ' .. name end }",
    /// )?;
    /// let mut vm = Rustia::builder().build()?;
    /// let exports = vm.install(&module)?;
    /// let greet = exports.function(&mut vm, "greet")?;
    /// let greeting: String = greet.call(&mut vm, ("Ada",))?;
    /// assert_eq!(greeting, "Hello, Ada");
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn install(&mut self, module: &CompiledModule) -> Result<Exports> {
        self.install_with_environment(module, None)
    }
}

impl<C> Rustia<C>
where
    C: HostSchema,
{
    fn install_with_environment(
        &mut self,
        module: &CompiledModule,
        environment: Option<rustia_vm::Value>,
    ) -> Result<Exports> {
        self.vm
            .begin_guest_transaction()
            .map_err(Error::from_engine)?;
        let outcome: Result<Exports> = (|| {
            let module_cohort = self.vm.create_module_cohort().map_err(Error::from_engine)?;
            let environment = match environment {
                Some(environment) => environment,
                None => self
                    .vm
                    .create_module_environment()
                    .map_err(Error::from_engine)?,
            };
            let value = self
                .vm
                .run_installation_with_environment(&module.program, environment, module_cohort)
                .map_err(Error::from_engine)?;
            let value = self.vm.retain_value(value).map_err(Error::from_engine)?;
            Ok(Exports::new(value, self.decision_epoch()))
        })();
        match outcome {
            Ok(exports) => {
                if let Err(error) = self.vm.commit_guest_transaction() {
                    self.vm.rollback_guest_transaction();
                    Err(Error::from_engine(error).installation_rolled_back())
                } else {
                    Ok(exports)
                }
            }
            Err(error) => {
                self.vm.rollback_guest_transaction();
                Err(error.installation_rolled_back())
            }
        }
    }
}

impl Rustia<()> {
    /// Registers a Rust function as a script global.
    ///
    /// Arguments and results are converted automatically. Return [`Result<T>`]
    /// from the callback to report a script-visible error. A call costs one fuel
    /// unit in addition to guest instructions; use [`add_fn_with`](Self::add_fn_with)
    /// to select another cost or declare captured-state effects.
    ///
    /// The callback must be `Fn + Send + Sync + 'static`. This helper declares
    /// it pure, so captured state must honour that promise. The registration
    /// survives [`reset_decision`](Self::reset_decision). For a live function
    /// value instead of a global binding, use [`create_function`](Self::create_function).
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// vm.add_fn("divide", |a: f64, b: f64| -> rustia::Result<f64> {
    ///     if b == 0.0 {
    ///         return Err(rustia::Error::embed("division by zero"));
    ///     }
    ///     Ok(a / b)
    /// })?;
    /// let quotient: f64 = vm.eval("return divide(84, 2)")?;
    /// assert_eq!(quotient, 42.0);
    /// # Ok::<(), rustia::Error>(())
    /// ```
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

    /// Registers a pure callback; equivalent to [`add_fn`](Self::add_fn).
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

    /// Registers a callback declared to read deterministic captured input.
    ///
    /// The callback receives ordinary arguments. For borrowed per-invocation
    /// input in a typed host, use [`add_input`](Self::add_input).
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

    /// Registers a callback declared to read captured slot state.
    ///
    /// For a borrowed [`Slots`] view in a typed host, use
    /// [`add_slot_read`](Self::add_slot_read).
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

    /// Registers a callback declared to produce slot-write effects.
    ///
    /// Captured Rust mutations are not rolled back. For a transactional
    /// [`SlotWrites`] journal, use [`add_slot_write`](Self::add_slot_write).
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

    /// Registers a callback declared to produce deterministic effects.
    ///
    /// Captured Rust mutations are not rolled back. For a transactional
    /// [`Effects`] journal, use [`add_effect`](Self::add_effect).
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

    /// Registers a callback declared to access mutable external state.
    ///
    /// Deterministic profiles reject this capability. For a borrowed [`HostMut`]
    /// view in a typed host, use [`add_external`](Self::add_external).
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

    /// Registers a callback declared to have nondeterministic effects.
    ///
    /// Deterministic profiles reject this capability. Captured Rust effects are
    /// outside installation and invocation rollback.
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

    /// Registers a callback with an explicit fuel cost and capability.
    ///
    /// A zero fuel cost or a capability forbidden by the selected profile fails
    /// before registration. The capability describes captured host behavior;
    /// it does not restrict what a Rust closure can do. See [`add_fn`](Self::add_fn)
    /// for the callback's conversion and lifetime rules.
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
        self.register_function(
            name,
            HostFunction::with_capability(capability, function).fuel_cost(fuel_cost),
        )
    }

    /// Installs a collection of host functions and modules atomically.
    ///
    /// Validation or binding failure leaves existing registrations unchanged.
    /// Use [`RustiaExtension`] to assemble a reusable application API.
    pub fn install_extension(&mut self, extension: RustiaExtension) -> Result<()> {
        extension.install(self)
    }
}

impl Default for Rustia {
    /// Creates the standard preset, matching [`Rustia::builder`].
    fn default() -> Self {
        Self::builder()
            .build()
            .expect("the standard preset is valid without caller-supplied configuration")
    }
}

/// A value returned by Rustia, or a script, conversion, or embedding [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// A compiled script ready for [`Rustia::install`].
///
/// Create it with [`Rustia::compile_module`]. It owns its source metadata and
/// compiled code independently of any interpreter, and remains reusable after
/// reset. A clone shares constant and string storage; function and debug metadata
/// are currently copied.
#[derive(Clone)]
pub struct CompiledModule {
    source_name: String,
    program: Program,
}

impl CompiledModule {
    /// Returns the source label supplied when this module was compiled.
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
    /// Returns the source label supplied when this entry was compiled.
    pub fn source_name(&self) -> &str {
        &self.source_name
    }
}

/// A half-open byte range in the source, `start..end`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    /// Byte offset of the first included byte.
    pub start: usize,
    /// Byte offset immediately after the range.
    pub end: usize,
}

impl SourceSpan {
    pub(crate) fn from_lexer(span: rustia_lexer::Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

/// The source label and one-based line number attached to an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation<'a> {
    /// Source label, such as `"rules.luau"`; it need not be a filesystem path.
    pub source: &'a str,
    /// One-based source line number.
    pub line: u32,
}

#[cfg(test)]
mod tests {
    use super::{
        finish_entry_lifecycle, Error, FromRustia, GuestSurface, IntoRustia, OpCapability,
        OwnedTable, OwnedValue, Profile, Result, Rustia, RustiaBuilder, RustiaExecutionProfile,
        RustiaExtension, RustiaHostFunctionInfo, ServiceClock, ServiceEntropy, SourceSpan,
        ValueType, VmErrorKind, RUSTIA_DECISION_RESET_ABI_VERSION,
        RUSTIA_DEFAULT_HOST_FUNCTION_FUEL, RUSTIA_HOST_FUEL_ABI_VERSION, RUSTIA_HOST_FUEL_POLICY,
        RUSTIA_MIN_HOST_FUNCTION_FUEL,
    };
    use rustia_compiler::{compile_with_options, CompileOptions};
    use rustia_parser::parse_source;
    use rustia_vm::{Vm, VmSettings};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::num::NonZeroU64;
    use std::sync::{Arc, Mutex};

    #[test]
    fn install_smoke() {
        let mut rustia = Rustia::new(Profile::DeterministicSim);
        let result = rustia.eval("local x: number = 6 * 7; return x").unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 42.0);
    }

    #[test]
    fn live_values_preserve_identity_cycles_and_rich_payloads() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let root = rustia
            .load(
                r#"
local root = {}
local richKey = {}
local shared = { answer = 42 }
root.self = root
root.left = shared
root.right = shared
root.binary = string.char(0, 255, 65)
root.bytes = buffer.fromstring(string.char(1, 0, 254))
root.direction = vector(1.5, -2, 3)
root[richKey] = shared
return root
"#,
            )
            .eval_live()
            .unwrap();
        assert_eq!(root.kind(&rustia).unwrap(), ValueType::Table);
        let table = root.as_table(&rustia).unwrap().unwrap();

        let self_value = table.get(&mut rustia, "self").unwrap();
        assert!(root.raw_equal(&mut rustia, &self_value).unwrap());
        let left = table.get(&mut rustia, "left").unwrap();
        let right = table.get(&mut rustia, "right").unwrap();
        assert!(left.raw_equal(&mut rustia, &right).unwrap());
        assert_eq!(
            table
                .get(&mut rustia, "binary")
                .unwrap()
                .string_bytes(&mut rustia)
                .unwrap(),
            Some(vec![0, 255, 65])
        );
        assert_eq!(
            table
                .get(&mut rustia, "bytes")
                .unwrap()
                .buffer_bytes(&mut rustia)
                .unwrap(),
            Some(vec![1, 0, 254])
        );
        assert_eq!(
            table
                .get(&mut rustia, "direction")
                .unwrap()
                .vector_components(&mut rustia)
                .unwrap(),
            Some([1.5, -2.0, 3.0])
        );

        table.set(&mut rustia, "added", 99_i32).unwrap();
        assert_eq!(
            table
                .get(&mut rustia, "added")
                .unwrap()
                .as_number(&rustia)
                .unwrap(),
            Some(99.0)
        );
        let rich_entry = table
            .entries(&mut rustia)
            .unwrap()
            .into_iter()
            .find(|(key, _)| key.kind(&rustia).unwrap() == ValueType::Table)
            .expect("table identity key");
        assert!(left.raw_equal(&mut rustia, &rich_entry.1).unwrap());

        let exports = rustia
            .load("local live = { answer = 7 }; return { live = live }")
            .install()
            .unwrap();
        let exported = exports.get_live(&mut rustia, "live").unwrap();
        assert_eq!(
            exported
                .as_table(&rustia)
                .unwrap()
                .unwrap()
                .get(&mut rustia, "answer")
                .unwrap()
                .as_number(&rustia)
                .unwrap(),
            Some(7.0)
        );

        let _: f64 = rustia.eval("collectgarbage('collect'); return 0").unwrap();
        assert_eq!(
            left.as_table(&rustia)
                .unwrap()
                .unwrap()
                .get(&mut rustia, "answer")
                .unwrap()
                .as_number(&rustia)
                .unwrap(),
            Some(42.0),
            "a live handle remains a collector root"
        );
        assert!(
            root.to_owned(&mut rustia).is_err(),
            "cyclic snapshots stay explicit"
        );

        let mut other = Rustia::new(Profile::ReproducibleStandard);
        assert!(matches!(
            root.kind(&other)
                .unwrap_err()
                .as_vm_error()
                .map(|error| &error.kind),
            Some(VmErrorKind::RetainedValueFromDifferentVm)
        ));
        other.reset_decision().unwrap();
        rustia.reset_decision().unwrap();
        assert!(matches!(
            root.kind(&rustia)
                .unwrap_err()
                .as_vm_error()
                .map(|error| &error.kind),
            Some(VmErrorKind::StaleRetainedValue {
                retained_epoch: 0,
                current_epoch: 1
            })
        ));
    }

    #[test]
    fn constructed_tables_install_into_the_live_globals_graph() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let globals = rustia.globals().unwrap();
        let root = rustia.create_table().unwrap();
        root.set(&mut rustia, "answer", 42_i32).unwrap();
        root.set_live(&mut rustia, "self", &root.as_value())
            .unwrap();
        globals
            .set_live(&mut rustia, "host_graph", &root.as_value())
            .unwrap();

        let returned = rustia.load("return host_graph").eval_live().unwrap();
        assert!(root.as_value().raw_equal(&mut rustia, &returned).unwrap());
        let returned_table = returned.as_table(&rustia).unwrap().unwrap();
        let returned_self = returned_table.get(&mut rustia, "self").unwrap();
        assert!(returned.raw_equal(&mut rustia, &returned_self).unwrap());

        // Ordinary execution does not invalidate unrelated live handles.
        root.set(&mut rustia, "after_run", 7_i32).unwrap();
        let sees_mutation: bool = rustia
            .eval("return host_graph.answer == 42 and host_graph.after_run == 7")
            .unwrap();
        assert!(sees_mutation);

        let globals_again = rustia.globals().unwrap();
        assert!(globals
            .as_value()
            .raw_equal(&mut rustia, &globals_again.as_value())
            .unwrap());
    }

    #[test]
    fn live_table_constructors_are_available_in_every_standard_surface_profile() {
        for profile in [
            Profile::LuauCompat,
            Profile::ReproducibleStandard,
            Profile::DeterministicSim,
            Profile::Puc51Compat,
        ] {
            let mut rustia = Rustia::new(profile);
            let value = rustia.create_table().unwrap();
            value.set(&mut rustia, "profile_value", 9_i32).unwrap();
            rustia
                .globals()
                .unwrap()
                .set_live(&mut rustia, "host_value", &value.as_value())
                .unwrap();
            let observed: f64 = rustia.eval("return host_value.profile_value").unwrap();
            assert_eq!(observed, 9.0, "profile {profile:?}");
        }
    }

    #[test]
    fn set_live_rejects_foreign_tables_and_values_before_mutation() {
        let mut first = Rustia::new(Profile::ReproducibleStandard);
        let mut second = Rustia::new(Profile::ReproducibleStandard);
        let first_table = first.create_table().unwrap();
        let second_table = second.create_table().unwrap();

        let foreign_value = first_table
            .set_live(&mut first, "foreign", &second_table.as_value())
            .expect_err("a value from another VM must reject");
        assert!(matches!(
            foreign_value.as_vm_error().map(|error| &error.kind),
            Some(VmErrorKind::RetainedValueFromDifferentVm)
        ));
        assert!(first_table
            .get(&mut first, "foreign")
            .unwrap()
            .is_nil(&first)
            .unwrap());

        let foreign_table = second_table
            .set_live(&mut first, "foreign", &first_table.as_value())
            .expect_err("a table from another VM must reject");
        assert!(matches!(
            foreign_table.as_vm_error().map(|error| &error.kind),
            Some(VmErrorKind::RetainedValueFromDifferentVm)
        ));
    }

    #[test]
    fn decision_reset_stales_constructor_handles_and_discards_global_mutations() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let globals = rustia.globals().unwrap();
        let table = rustia.create_table().unwrap();
        globals
            .set_live(&mut rustia, "decision_local", &table.as_value())
            .unwrap();

        rustia.reset_decision().unwrap();
        assert!(matches!(
            globals
                .get(&mut rustia, "decision_local")
                .unwrap_err()
                .as_vm_error()
                .map(|error| &error.kind),
            Some(VmErrorKind::StaleRetainedValue {
                retained_epoch: 0,
                current_epoch: 1
            })
        ));
        assert!(matches!(
            table
                .set(&mut rustia, "value", 1_i32)
                .unwrap_err()
                .as_vm_error()
                .map(|error| &error.kind),
            Some(VmErrorKind::StaleRetainedValue {
                retained_epoch: 0,
                current_epoch: 1
            })
        ));
        let discarded: bool = rustia.eval("return decision_local == nil").unwrap();
        assert!(discarded);
    }

    #[test]
    fn hardened_leaves_reject_live_table_authority_without_allocating() {
        let mut rustia = RustiaBuilder::deterministic_leaf()
            .memory_limit_bytes(512 * 1024)
            .build_for_host::<()>()
            .unwrap();
        let before = rustia.memory_stats().live_bytes();

        let globals = rustia.globals().expect_err("hardened globals must reject");
        assert!(matches!(
            globals,
            Error { kind: crate::ErrorKind::Vm(ref error), .. }
                if matches!(
                    &error.kind,
                    VmErrorKind::HardenedLeafViolation {
                        feature: "mutable globals handles"
                    }
                )
        ));
        let table = rustia
            .create_table()
            .expect_err("hardened table construction must reject");
        assert!(matches!(
            table,
            Error { kind: crate::ErrorKind::Vm(ref error), .. }
                if matches!(
                    &error.kind,
                    VmErrorKind::HardenedLeafViolation {
                        feature: "live table construction"
                    }
                )
        ));
        assert_eq!(rustia.memory_stats().live_bytes(), before);
    }

    #[test]
    fn live_table_constructor_quota_failure_recovers_after_a_root_is_dropped() {
        let mut baseline = RustiaBuilder::standard().build().unwrap();
        let limit = baseline
            .memory_stats()
            .live_bytes()
            .saturating_add(4 * 1024);
        drop(baseline);
        let mut rustia = RustiaBuilder::standard()
            .memory_limit_bytes(limit)
            .build()
            .unwrap();
        let mut retained = Vec::new();
        let error = loop {
            match rustia.create_table() {
                Ok(table) => {
                    retained.push(table);
                    assert!(retained.len() < 10_000, "constructor quota did not bind");
                }
                Err(error) => break error,
            }
        };
        assert!(matches!(
            error,
            Error { kind: crate::ErrorKind::Vm(ref error), .. } if matches!(&error.kind, VmErrorKind::OutOfMemory)
        ));

        retained.pop().expect("at least one table fit before OOM");
        rustia
            .create_table()
            .expect("dropping one root must make one constructor admission recover");
    }

    #[test]
    fn live_functions_accept_and_return_live_values() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let function = rustia
            .load(
                "return function(value) local t = { value = value }; t.self = t; return value, t end",
            )
            .eval_live()
            .unwrap()
            .as_function(&rustia)
            .unwrap()
            .unwrap();
        let argument = rustia.load("return { answer = 42 }").eval_live().unwrap();
        let results = function
            .call_values(&mut rustia, std::slice::from_ref(&argument))
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(argument.raw_equal(&mut rustia, &results[0]).unwrap());
        let returned = results[1].as_table(&rustia).unwrap().unwrap();
        let returned_value = returned.get(&mut rustia, "value").unwrap();
        assert!(argument.raw_equal(&mut rustia, &returned_value).unwrap());
        let returned_self = returned.get(&mut rustia, "self").unwrap();
        assert!(returned
            .as_value()
            .raw_equal(&mut rustia, &returned_self)
            .unwrap());
    }

    #[test]
    fn entry_lifecycle_removes_new_records_when_reset_fails() {
        let mut records = vec![1, 2, 3];
        let result = finish_entry_lifecycle(
            &mut records,
            2,
            Ok(Ok(42)),
            Err(Error::embed("reset failed".to_owned())),
        );
        assert!(
            matches!(result, Err(Error { kind: crate::ErrorKind::Embed(ref message), .. }) if message == "reset failed")
        );
        assert_eq!(records, [1, 2]);
    }

    #[test]
    fn compiled_modules_can_be_cloned_across_vms() {
        let module = Rustia::compile_module(
            "shared.luau",
            "local greeting = 'compiled string shared across VMs'; return { greeting = greeting }",
        )
        .unwrap();
        let mut first = Rustia::new(Profile::ReproducibleStandard);
        let mut second = Rustia::new(Profile::ReproducibleStandard);

        first.install(&module).unwrap();
        second.install(&module).unwrap();
    }

    #[test]
    fn typed_run_converts_owned_results() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia.eval("return 40 + 2").unwrap();
        let numbers: Vec<i32> = rustia.eval("return { 1, 2, 3 }").unwrap();
        let missing: Option<String> = rustia.eval("return nil").unwrap();
        let named: BTreeMap<String, i32> = rustia.eval("return { scout = 5, brute = 9 }").unwrap();

        assert_eq!(answer, 42.0);
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(missing, None);
        assert_eq!(named.get("scout"), Some(&5));
        assert_eq!(named.get("brute"), Some(&9));
    }

    #[test]
    fn typed_run_reports_conversion_errors() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia.eval::<String>("return 42").unwrap_err().to_string();
        assert!(error.contains("expected string, got number"), "{error}");

        let array_error = rustia
            .eval::<BTreeMap<String, i32>>("return { 1, 2 }")
            .unwrap_err()
            .to_string();
        assert!(
            array_error.contains("expected string-keyed table, got table with array entries"),
            "{array_error}"
        );

        let key_error = rustia
            .eval::<BTreeMap<String, i32>>("return { [true] = 1 }")
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
            .eval(
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
            let Err(Error {
                kind: crate::ErrorKind::Vm(error),
                ..
            }) = rustia.eval::<OwnedValue>(source)
            else {
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
        let value: OwnedValue = rustia
            .eval(
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
        let labels = OwnedTable::string_entries([("kind", "scout"), ("role", "ai")]).unwrap();
        assert_eq!(
            labels.get_string_as::<String>("kind").unwrap().as_deref(),
            Some("scout")
        );
        assert_eq!(labels.get_string_as::<String>("missing").unwrap(), None);

        let stats = OwnedTable::string_entries([("hp", 5_i32), ("speed", 7_i32)]).unwrap();
        assert_eq!(stats.get_string_as::<i32>("hp").unwrap(), Some(5));

        let error = labels
            .get_string_as::<bool>("kind")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected boolean, got string"),
            "unexpected error: {error}"
        );

        let array = OwnedTable::array_values([1_i32, 2, 3]).unwrap();
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
    fn integer_embedding_rejects_silent_luau_number_rounding() {
        let exact = (1_u64 << 53).into_rustia().unwrap();
        assert_eq!(exact, OwnedValue::Number((1_u64 << 53) as f64));

        for error in [
            ((1_u64 << 53) + 1).into_rustia().unwrap_err(),
            u64::MAX.into_rustia().unwrap_err(),
            i64::MAX.into_rustia().unwrap_err(),
        ] {
            assert!(
                error.to_string().contains("cannot be represented exactly"),
                "unexpected error: {error}"
            );
        }

        assert!(u64::from_rustia(OwnedValue::Number(2.0_f64.powi(64))).is_err());
        assert!(i64::from_rustia(OwnedValue::Number(2.0_f64.powi(63))).is_err());

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        assert!(rustia.insert_global("too_wide", u64::MAX).is_err());
    }

    #[test]
    fn serde_integer_conversion_is_exact_for_values_and_map_keys() {
        assert_eq!(
            OwnedValue::from_serialize(&(1_u64 << 53)).unwrap(),
            OwnedValue::Number((1_u64 << 53) as f64)
        );
        assert!(OwnedValue::from_serialize(&((1_u64 << 53) + 1)).is_err());
        assert!(OwnedValue::from_serialize(&u128::MAX).is_err());

        let exact_wide = 1_i128 << 100;
        let serialized = OwnedValue::from_serialize(&exact_wide).unwrap();
        assert_eq!(serialized.deserialize::<i128>().unwrap(), exact_wide);

        let rounded_key = BTreeMap::from([((1_u64 << 53) + 1, true)]);
        assert!(OwnedValue::from_serialize(&rounded_key).is_err());

        assert!(OwnedValue::Number(2.0_f64.powi(64))
            .deserialize::<u64>()
            .is_err());
        assert!(OwnedValue::Number(2.0_f64.powi(128))
            .deserialize::<u128>()
            .is_err());
    }

    #[test]
    fn owned_table_helpers_support_nested_typed_access() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let value: OwnedValue = rustia
            .eval(
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
        let value: OwnedValue = rustia
            .eval(
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
            .insert_global(
                "named",
                OwnedTable::string_entries([("kind", "scout")]).unwrap(),
            )
            .unwrap();

        let value = rustia
            .eval(
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

        let result = rustia.eval("return clamp(12 + difficulty, 0, 10)").unwrap();
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

        let result = rustia.eval("return sum8(1, 2, 3, 4, 5, 6, 7, 8)").unwrap();
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
            .eval(
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
            .eval("return pure() + input() + slot() + slot_write(1) + effect(3)")
            .unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 21.0);
    }

    #[test]
    fn deterministic_facade_rejects_external_or_nondeterministic_helpers() {
        let mut external = Rustia::new(Profile::DeterministicSim);
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = external.add_external_state_fn("clock", || 1.0)
        else {
            panic!("expected deterministic profile to reject external-state host function");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let mut nondeterministic = Rustia::new(Profile::DeterministicSim);
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = nondeterministic.add_nondeterministic_fn("random", || 1.0)
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

        let result = rustia.eval("return clock() + random()").unwrap();
        assert_eq!(f64::from_rustia(result).unwrap(), 18.0);
    }

    #[test]
    fn typed_host_function_arg_errors_include_function_and_position() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        rustia
            .add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))
            .unwrap();

        let wrong_type = rustia
            .eval::<OwnedValue>("return clamp('bad', 0, 10)")
            .unwrap_err();
        let wrong_type = wrong_type.to_string();
        assert!(
            wrong_type.contains("host function `clamp` argument 1: expected number, got string"),
            "{wrong_type}"
        );

        let missing_arg = rustia.eval::<OwnedValue>("return clamp(4, 0)").unwrap_err();
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
                    return Err(Error::embed("division by zero".to_owned()));
                }
                Ok(x / y)
            })
            .unwrap();

        let value: f64 = rustia.eval("return checked_div(12, 3)").unwrap();
        assert_eq!(value, 4.0);

        let error = rustia
            .eval::<OwnedValue>("return checked_div(12, 0)")
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
            .load(
                "\
return {
    label = 'ai',
    decide = function(x)
        return x + 1
    end,
}
",
            )
            .install()
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
            .load(
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
            .install()
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
            .load(
                "\
return {
    rotate = function(a, b, c, d, e, f, g, h)
        return h, g, f, e, d, c, b, a
    end,
}
",
            )
            .install()
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

        let result = rustia.eval("return host.bias() + host.emit(4)").unwrap();
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
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
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
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
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
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
            panic!("expected deterministic profile to reject external-state extension");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn failed_replacement_install_keeps_old_function_callable() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .load("return { decide = function() return 1 end }")
            .source_name("ai.luau")
            .install()
            .unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();

        assert!(rustia
            .load("return {")
            .source_name("ai.luau")
            .install()
            .is_err());
        let result: f64 = decide.call(&mut rustia, ()).unwrap();
        assert_eq!(result, 1.0);
    }

    #[test]
    fn successful_replacement_install_keeps_both_versions_callable() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let first_exports = rustia
            .load("return { label = 'old', decide = function() return 1 end }")
            .source_name("ai.luau")
            .install()
            .unwrap();
        let first_decide = first_exports.function(&mut rustia, "decide").unwrap();

        let second_exports = rustia
            .load("return { label = 'new', decide = function() return 2 end }")
            .source_name("ai.luau")
            .install()
            .unwrap();
        let second_decide = second_exports.function(&mut rustia, "decide").unwrap();

        assert_eq!(
            first_exports.get::<String>(&mut rustia, "label").unwrap(),
            "old"
        );
        assert_eq!(
            second_exports.get::<String>(&mut rustia, "label").unwrap(),
            "new"
        );

        let old_result: f64 = first_decide.call(&mut rustia, ()).unwrap();
        let new_result: f64 = second_decide.call(&mut rustia, ()).unwrap();
        assert_eq!(old_result, 1.0);
        assert_eq!(new_result, 2.0);
    }

    #[test]
    fn retained_function_root_survives_after_exports_are_dropped() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .load(
                "local state = { answer = 42 }; \
                 return { read = function() return state.answer end }",
            )
            .install()
            .unwrap();
        let read = exports.function(&mut rustia, "read").unwrap();
        drop(exports);

        let _: f64 = rustia.eval("collectgarbage('collect'); return 0").unwrap();
        let answer: f64 = read.call(&mut rustia, ()).unwrap();
        assert_eq!(answer, 42.0);
    }

    #[test]
    fn retained_function_root_survives_repeated_slot_reuse() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let exports = rustia
            .load(
                "local state = { answer = 42 }; \
                 return { read = function() return state.answer end }",
            )
            .install()
            .unwrap();
        let read = exports.function(&mut rustia, "read").unwrap();
        drop(exports);

        for _ in 0..32 {
            let _: f64 = rustia
                .eval(
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
            .load("return { read = function() return 42 end }")
            .install()
            .unwrap();
        let read = exports.function(&mut first, "read").unwrap();
        let mut second = Rustia::new(Profile::ReproducibleStandard);

        let exports_error = exports.value(&mut second).unwrap_err();
        assert!(matches!(
            exports_error.as_vm_error().map(|error| &error.kind),
            Some(VmErrorKind::RetainedValueFromDifferentVm)
        ));
        let function_error = read.call::<_, f64>(&mut second, ()).unwrap_err();
        assert!(matches!(
            function_error.as_vm_error().map(|error| &error.kind),
            Some(VmErrorKind::RetainedValueFromDifferentVm)
        ));
    }

    #[test]
    fn named_compile_preserves_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let error = rustia
            .load("local t = {}; t[0 / 0] = 1")
            .source_name("bad_ai.luau")
            .install()
            .unwrap_err();
        assert!(error.to_string().contains("bad_ai.luau:1:"), "{error}");
    }

    #[test]
    fn chunk_builder_preserves_source_name_in_runtime_diagnostics() {
        let mut rustia = Rustia::new(Profile::ReproducibleStandard);

        let answer: f64 = rustia
            .load("return 40 + 2")
            .source_name("inline.luau")
            .eval()
            .unwrap();
        assert_eq!(answer, 42.0);

        let owned = rustia
            .load("return 'ok'")
            .source_name("owned.luau")
            .eval()
            .unwrap();
        assert_eq!(String::from_rustia(owned).unwrap(), "ok");

        let exports = rustia
            .load("return { answer = 42 }")
            .source_name("exports.luau")
            .install()
            .unwrap();
        assert_eq!(exports.get::<i32>(&mut rustia, "answer").unwrap(), 42);

        let error = rustia
            .load("local t = {}; t[0 / 0] = 1")
            .source_name("bad_inline.luau")
            .eval::<f64>()
            .unwrap_err();
        assert!(error.to_string().contains("bad_inline.luau:1:"), "{error}");
    }

    #[test]
    fn public_errors_expose_source_span_and_runtime_location() {
        let Err(error) = Rustia::compile_module("compile-diag.luau", "break") else {
            panic!("break outside loop should fail compilation");
        };
        assert_eq!(error.source_span(), Some(SourceSpan { start: 0, end: 5 }));
        assert_eq!(
            error.source_location(),
            Some(crate::SourceLocation {
                source: "compile-diag.luau",
                line: 1
            })
        );

        let mut rustia = Rustia::new(Profile::ReproducibleStandard);
        let Err(error) = rustia
            .load(
                "\
local t = {}
t[0 / 0] = 1
",
            )
            .source_name("runtime-diag.luau")
            .eval::<OwnedValue>()
        else {
            panic!("NaN table key should fail at runtime");
        };
        let location = error
            .source_location()
            .expect("runtime error should carry source location");
        assert_eq!(location.source, "runtime-diag.luau");
        assert_eq!(location.line, 2);
        assert!(error.source_span().is_some());
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
            .load(
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
            .install()
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
        let second_spec: ScenarioSpec = exports.deserialize(&mut rustia).unwrap();
        assert_eq!(second_spec, spec);

        rustia.insert_serde("config", &spec).unwrap();
        let value = rustia
            .eval("return config.enemies[2].kind .. ':' .. tostring(config.difficulty)")
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
            .eval("return host.fixed_mul(6, 7) + host.emit(1)")
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
        let mut vm = Vm::new(Profile::ReproducibleStandard.to_core());
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
        let mut vm = Vm::new(Profile::ReproducibleStandard.to_core());
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
            .eval(
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
            .eval(
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
            .eval(
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
            .eval(
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
            .eval(
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
            .eval(
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
            .eval(
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
            .eval(
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
        let captured = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&captured);
        let mut rustia = RustiaBuilder::deterministic()
            .stdout(move |text| sink.lock().unwrap().push_str(text))
            .build()
            .unwrap();

        rustia.exec("print('hello', 42)").unwrap();
        assert_eq!(captured.lock().unwrap().as_str(), "hello\t42\n");
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
            (
                RustiaExecutionProfile::new(Profile::Puc51Compat, GuestSurface::Standard),
                7,
                [1, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                RustiaExecutionProfile::new(Profile::Puc51Compat, GuestSurface::HardenedLeaf),
                8,
                [1, 0, 0, 0, 4, 0, 0, 0, 2, 0, 0, 0],
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
                guest_surface: GuestSurface::HardenedLeaf.to_core(),
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

        let time: f64 = rustia.eval("return os.time()").unwrap();
        assert_eq!(time, 1.5);
        let surface: bool = rustia
            .eval(
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
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
            panic!("expected deterministic profile to reject a nondeterministic wall clock");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let result = RustiaBuilder::deterministic()
            .monotonic_clock(ServiceClock::monotonic())
            .build();
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
            panic!("expected deterministic profile to reject a nondeterministic monotonic clock");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));

        let result = RustiaBuilder::deterministic()
            .entropy(ServiceEntropy::system())
            .build();
        let Err(Error {
            kind: crate::ErrorKind::Vm(error),
            ..
        }) = result
        else {
            panic!("expected deterministic profile to reject nondeterministic entropy");
        };
        assert!(matches!(error.kind, VmErrorKind::CapabilityRemoved { .. }));
    }

    #[test]
    fn fixed_entropy_is_reproducible_and_changes_the_initial_random_sequence() {
        let sequence = |seed| {
            let mut rustia = RustiaBuilder::standard()
                .entropy(ServiceEntropy::fixed(seed))
                .build()
                .unwrap();
            [
                rustia.eval::<f64>("return math.random()").unwrap(),
                rustia.eval::<f64>("return math.random()").unwrap(),
            ]
        };

        assert_eq!(sequence(42), sequence(42));
        assert_ne!(sequence(42), sequence(43));
        assert_eq!(sequence(0), {
            let mut rustia = RustiaBuilder::standard().build().unwrap();
            [
                rustia.eval::<f64>("return math.random()").unwrap(),
                rustia.eval::<f64>("return math.random()").unwrap(),
            ]
        });
    }

    #[test]
    fn standard_preset_keeps_the_pinned_epoch_and_logical_clock_by_default() {
        let mut rustia = Rustia::builder().build().unwrap();

        let time: f64 = rustia.eval("return os.time()").unwrap();
        assert_eq!(time, 1_700_000_000.0);
        let logical: bool = rustia
            .eval(
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
        let module = Rustia::compile_module(
            "decision.luau",
            "return { answer = 42, decide = function(x) return x + 1 end }",
        )
        .unwrap();
        let mut rustia = RustiaBuilder::deterministic().build().unwrap();
        let exports = rustia.install(&module).unwrap();
        let decide = exports.function(&mut rustia, "decide").unwrap();
        let exports_clone = exports.clone();
        let decide_clone = decide.clone();

        assert_eq!(exports.decision_epoch(), 0);
        assert_eq!(decide.decision_epoch(), 0);
        assert_eq!(decide.call::<_, f64>(&mut rustia, (4.0,)).unwrap(), 5.0);

        rustia.reset_decision().unwrap();
        assert_eq!(rustia.decision_epoch(), 1);
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

        let fresh = rustia.install(&module).unwrap();
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
            .raw_settings(settings)
            .wall_clock(ServiceClock::fixed(1_234.0))
            .extension(extension)
            .build()
            .unwrap();
        rustia
            .add_pure_fn("double", |value: f64| value * 2.0)
            .unwrap();
        rustia.insert_global("input", 5.0).unwrap();

        let configured: bool = rustia
            .eval(
                "\
decision_global = (decision_global or 0) + 1
return input == 5 and decision_global == 1
    and host.bias() == 7 and double(3) == 6 and os.time() == 1234
",
            )
            .unwrap();
        assert!(configured);
        let first_random: Vec<f64> = rustia
            .eval("return { math.random(), math.random(), math.random() }")
            .unwrap();
        assert!(rustia.vm().deterministic_collection_count() > 0);
        let peak_before_reset = rustia.memory_stats().peak_live_bytes().unwrap();

        rustia.reset_decision().unwrap();
        assert_eq!(rustia.vm().deterministic_collection_count(), 0);
        assert_eq!(
            rustia.memory_stats().peak_live_bytes(),
            Some(peak_before_reset),
            "lifetime peak telemetry must survive a decision reset"
        );
        let clean_and_configured: bool = rustia
            .eval(
                "\
return input == nil and decision_global == nil
    and host.bias() == 7 and double(3) == 6 and os.time() == 1234
",
            )
            .unwrap();
        assert!(clean_and_configured);
        let second_random: Vec<f64> = rustia
            .eval("return { math.random(), math.random(), math.random() }")
            .unwrap();
        assert_eq!(first_random, second_random);
        assert!(rustia.vm().deterministic_collection_count() > 0);
    }

    #[test]
    fn decision_reset_reaches_a_many_decision_memory_plateau() {
        let module = Rustia::compile_module(
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
            let exports = rustia.install(&module).unwrap();
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
        let clean: bool = rustia.eval("return decision_global == nil").unwrap();
        assert!(clean);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_preset_installs_system_clocks_and_the_stock_loadstring_compiler() {
        let mut rustia = RustiaBuilder::live().build().unwrap();

        let time: f64 = rustia.eval("return os.time()").unwrap();
        assert!(
            time >= 1_700_000_000.0,
            "live() os.time() should read the real wall clock, got {time}"
        );
        let clock: f64 = rustia.eval("return os.clock()").unwrap();
        assert!(
            clock > 0.0,
            "live() os.clock() should read the real monotonic clock, got {clock}"
        );
        let answer: f64 = rustia.eval("return loadstring('return 40 + 2')()").unwrap();
        assert_eq!(answer, 42.0);
        // print goes to real process stdout via `print!`, which a test cannot read back from
        // inside the process; sink delivery is covered by the VM-level capture tests.
    }
}
