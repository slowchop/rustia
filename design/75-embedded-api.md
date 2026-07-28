# rustia - embedded API goal and plan

> **Status: [TYPED INVOCATION + DECISION RESET IMPLEMENTED - advanced interop pending] (updated 2026-07-28).**
> This records the public Rust embedding API direction and its honest implementation boundary after comparing
> rustia's constraints with current Rust scripting/runtime APIs. It refines the locked
> architecture axis H in [`30-architecture.md`](30-architecture.md) and the grabby bridge contract in
> [`70-host-bridge-contract.md`](70-host-bridge-contract.md).

## Goal Name

**Grabby-Compatible Embedded API**

This is the API goal, not just a feature label. It means rustia must provide a general-purpose, public Rust
embedding surface that is pleasant outside grabby, while also being strong enough for grabby's deterministic
per-agent AI loop without leaking grabby-specific types into public rustia.

## Current boundary

Typed run/compile helpers, owned scalar and table values, typed host functions, extension metadata/capabilities,
retained `Function`/`Exports` handles with install-generation checks, owned serde, hot reload, diagnostics,
early declarative macro sugar, and the runtime output/time services slice (environment presets, a stdout sink,
and determinism-labeled clock providers per [`72-host-services.md`](72-host-services.md)) are implemented.
The concrete safe-Rust host seam is also implemented: `HostSchema` separates persistent registrations from a
fresh, possibly borrowing `HostContext` for each typed invocation; callbacks receive capability-specific views;
all guest calls in the invocation share fuel; and effects/slot writes stage into a bounded, ordered,
record-transactional journal.
The authoritative-leaf layer from
[`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md) is also implemented:
`RustiaBuilder::deterministic_leaf()` selects a fail-closed `DeterministicSim` guest allowlist,
`Rustia::compile_entry`/`Invocation::call_entry` avoid retained module/export state in the hot path, and the
specialized hardened reset preserves only the sealed immutable configuration prefix.
Object/program reclamation, real VM-owned live/reserved/peak telemetry, and the correctness-first logical-live
quota/atomic OOM path are now implemented and validated by the exact-commit `safe-memory-588b541` native Linode
cohort plus its wasm32 512 KiB quota/recovery probe. The next release-critical slice is not more facade
decoration: the epoch-safe decision reset is now implemented; remaining work is O(1) quota accounting, a cheap
host-owned module resolver/cache, transactional installation/reload failures, rich graph-preserving values, and
live zero-copy serde.

## Non-Negotiable Requirements

- Koto-shaped migration path: construct VM, configure limits/fuel, register host functions, inject globals,
  compile/run chunks, read exports, call script functions from Rust.
- Luau-compatible script semantics remain the runtime contract.
- Public API must not leak gc-arena lifetimes or internal `Value` representation.
- Public rustia contains no `SimId`, `EffectRecord`, `ControlInput`, or grabby-specific enum.
- DeterministicSim allows deterministic host seams, not arbitrary host mutation.
- Cross-tick authoritative state for grabby lives in host typed slots, never Lua globals/upvalues.
- Rustia assigns one dense record sequence per successful invocation; the grabby bridge adds `emitter`, orders
  by `(emitter, seq)`, then commits.
- Q16.16/int boundary is the deterministic host boundary; public rustia should make this ergonomic but generic.

## API Research Summary

The best existing Rust APIs converge on a few patterns:

- **mlua**: richest scripting API precedent. `IntoLua`/`FromLua`, multi-value conversion, `UserData`, scoped
  callbacks, serde, async, optional Send mode.
- **rhai**: simplest function-registration DX. `Engine::register_fn` makes plain Rust functions callable from
  scripts.
- **koto**: direct migration precedent. `Koto` facade, settings, prelude insertion, compile/run, exports, serde.
- **rquickjs**: clean runtime/context split, `IntoJs`/`FromJs`, persistent handles, loader and async variants.
- **boa**: context/eval model plus native function/object macros.
- **rune**: strong compile unit + runtime + VM split, rich diagnostics, hot reload, typed `Vm::call`.
- **roto**: domain-specific typed embedding, registered Rust types/functions, hot reload, JIT; too static for
  Luau but useful for typed host-boundary ideas.
- **starlark**: deterministic embedding precedent: globals/module/evaluator split, temp/frozen heaps, native
  function macros, explicit function calls from Rust.
- **wasmtime/wasmi**: best linker pattern: named host imports, typed `func_wrap`, store data, reusable linker.
- **deno_core**: most advanced op layer: macro-generated ops, fastcalls, fallible ops, async modes, typed
  conversion, extension packaging.

## Best Five Implementation Shapes

### 1. Layered koto-shaped facade over typed conversions

This is the primary public API.

```rust
let mut vm = Rustia::builder()
    .profile(Profile::DeterministicSim)
    .fuel(50_000)
    .build()?;

vm.add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))?;
vm.insert_global("difficulty", 3)?;

let quick: f64 = vm.run("return clamp(difficulty + 9, 0, 10)")?;
let exports = vm.compile_and_run_exports("return { decide = function(x) return clamp(x, 0, 10) end }")?;
let decide = exports.function(&mut vm, "decide")?;
let y: f64 = decide.call(&mut vm, (12.0,))?;
```

Core pieces:

- typed `Rustia::run<T>()` for direct single-result conversion.
- source-named `run_named<T>()`, `run_owned_named()`, and compile/run helpers for host-controlled diagnostics.
- public error accessors for compile-time byte spans and runtime source/line locations.
- `OwnedValue` as the public value firewall.
- `IntoRustia` / `FromRustia`.
- `IntoRustiaMulti` / `FromRustiaMulti`.
- owned conversions for `Option<T>`, `Vec<T>`, deterministic `BTreeMap<String, T>`, direct `OwnedTable`
  values, typed `OwnedTable` helper constructors/getters, and nested table/array access.
- typed host functions support fallible `rustia::Result<T>` returns.
- typed host functions and retained script calls support tuple-shaped arguments/results through arity 8.
- retained `Function` and `Exports` handles with generation freshness checks.
- internal gc-arena `Value` remains behind the facade.

Runtime services are part of the same builder (implemented per [`72-host-services.md`](72-host-services.md)):
`RustiaBuilder::standard()` (= `Rustia::builder()`) adds a real stdout print sink on native targets while
keeping deterministic clock defaults; `deterministic()` is the bare `DeterministicSim` world; `live()` is
"normal Luau" with system clocks and the stock `loadstring` compiler, and fails loudly on wasm32. The
`.stdout(...)`/`.wall_clock(...)`/`.monotonic_clock(...)` sockets take a sink closure or a `ServiceClock`
(`system()` is nondeterministic, `fixed(t)` deterministic, `from_fn(f)` host-asserted); with no provider
plugged, no-argument `os.time`/`os.date` read a pinned epoch and `os.clock()` reads a deterministic logical
clock (executed instructions / 1_000_000). On `DeterministicSim` the `os` table is registered iff a
deterministic clock provider is plugged, and `build()` rejects a nondeterministic one.

Reason: best fit for koto migration, grabby ergonomics, and public API clarity.

### 2. Extension/linker layer for serious hosts

This is the grabby integration point and the scalable public host-module API.

```rust
struct GrabbyHost;

impl HostSchema for GrabbyHost {
    type Input = AgentInput;
    type Slots = [i64];
    type External = ();
    type Effect = Effect;
    type SlotWrite = (u8, i64);
    type Record = EffectRecord;
}

let bridge = HostModule::<GrabbyHost>::new("grabby")
    .pure("fixed_mul", 2, fixed_mul)
    .input("target_x", 2, |input: Input<'_, GrabbyHost>| input.get().target_x)
    .slot_read("slot", 2, |slots: Slots<'_, GrabbyHost>, slot: usize| {
        slots.get()[slot]
    })
    .slot_write(
        "set_slot",
        3,
        |mut writes: SlotWrites<'_, GrabbyHost>, slot: u8, value: i64| {
            writes.write((slot, value))
        },
    )
    .effect(
        "move",
        3,
        |mut effects: Effects<'_, GrabbyHost>, dx: i32, dy: i32| {
            effects.emit(Effect::Move { dir: [dx, dy] })
        },
    );

let mut vm = RustiaBuilder::deterministic().build_for_host::<GrabbyHost>()?;
vm.install_host_module(bridge)?;
```

Each registration carries:

- name and optional module namespace.
- typed signature / arity metadata.
- fuel cost.
- capability, from which the VM derives profile policy.

The current VM capability set is:

- `Pure`
- `ReadsDeterministicInput`
- `ReadsSlotState`
- `EmitsDeterministicEffect`
- `WritesSlotEffect`
- `ReadsExternalMutableState`
- `NondeterministicEffect`

`DeterministicSim` allows the first five and rejects the last two; the Standard profiles allow all seven.

The typed surface derives capability from the registration method and callback's first parameter:

| registration | callback shape |
|---|---|
| `pure` | `|args...| -> T` or `rustia::Result<T>`; no context authority |
| `input` | `|input: Input<'_, H>, args...| -> T` or `rustia::Result<T>` |
| `slot_read` | `|slots: Slots<'_, H>, args...| -> T` or `rustia::Result<T>` |
| `effect` | `|effects: Effects<'_, H>, args...| -> T` or `rustia::Result<T>` |
| `slot_write` | `|writes: SlotWrites<'_, H>, args...| -> T` or `rustia::Result<T>` |
| `external` / `nondeterministic` | `|host: HostMut<'_, H>, args...| -> T` or `rustia::Result<T>` |

The last two are rejected by `DeterministicSim`. `HostModule::namespace()`, `functions()`, and `fuel_policy()`
expose pre-install namespace/signature/capability/fuel metadata, and namespaced installation is prevalidated so
a rejected module retains no typed dispatcher bindings. The context-free `Rustia<()>` facade and
`RustiaExtension` remain the convenient legacy surface; `Rustia<H>` is the concrete context-bearing surface.

`HostSchema` is `'static` because the VM stores functions against it. `HostContext<H>` is a separate trait
object borrowed only for one call to `invoke`/`invoke_into`, so its concrete implementation may carry unrelated
block-local lifetimes on successive ticks. Callback closures are still `Fn + Send + Sync + 'static`; the
capability views themselves are short-lived and cannot escape. This removes the need to capture worker state,
but it does not prove semantic purity: a callback can still read globals or mutate captured/interior state.
That capture behavior is a documented embedding trust boundary.

The schema's associated types have narrow roles: `Input` is immutable decision input, `Slots` is the immutable
persistent-slot snapshot, `External` is mutable live authority available only through `HostMut`, `Effect` and
`SlotWrite` are callback-produced commands, and `Record` is their common ordered publication type.
`HostContext::input`/`slots` project the first two, `external_mut` projects the third,
`record_effect`/`record_slot_write` add the supplied dense sequence, and `record_bytes` returns the stable
logical protocol size charged to the byte budget.

#### Concrete invocation and record transaction

Typed VMs run guest module top-level code and retained functions only through `Invocation<'call, H>`.
`InvocationLimits::new(fuel).with_max_records(max_records).with_max_record_bytes(max_record_bytes)` names the
three limits and creates one monotonic budget shared by every `Invocation::install`/`run`,
`Function::call_in`, nested guest/metamethod call, and host dispatch performed by the outer Rust closure. The
builder/`VmSettings` initial fuel, when configured, is a VM-wide ceiling; the effective typed budget is the
lower of that ceiling and `InvocationLimits::fuel()`. The
output bounds are checked before a record enters the journal; effects and slot writes share a single dense
sequence starting at zero. `HostContext::record_bytes` defines the deterministic logical payload charge.

There are two record checkpoints. Each host handler rolls back only the suffix it appended when it returns an
error or panics. The outer `invoke`/`invoke_into` rolls back every record when its operation returns an error,
including a guest failure or Rust result-conversion failure. Guest protection follows normal `pcall` structure:
a completed host call survives a later guest error caught by `pcall`; a failing handler's own partial suffix
does not.

Fuel exhaustion, OOM, record-count or record-byte overflow, an invalid boundary yield, a missing typed-host
dispatcher, and host panic are hard transaction aborts: guest `pcall` and callbacks cannot turn them into a
successful publication. The extra hard OOM/yield treatment is local to typed transactional execution, rather
than a global change to ordinary legacy execution. Host panics are caught at the VM host-call boundary; records
roll back and every later guest execution entry point rejects until `reset_decision()` succeeds.
Containment covers typed/context-free host functions, extension ops, stdout and clock providers, and the
configured `loadstring` compiler. This requires `panic = "unwind"`; an
aborting build terminates or traps before Rustia can intervene, so callbacks in
that configuration must not panic.

The outer closure passed to `invoke` is ordinary caller-side Rust control flow. Its panic (or one from a
caller-side argument/result conversion) continues unwinding after dropping the unpublished journal; it is not
converted to `HostPanicked` and does not poison an otherwise idle VM. Conversions performed inside a
guest-invoked host callback are inside the containment boundary and do poison on panic.
The hardened `invoke_entry_into` lifecycle catches a caller-side conversion
unwind, resets the decision, and resumes the same panic.

This is intentionally a **record transaction**, not a heap/context transaction. It does not undo guest
globals/tables, mutation through profile-gated `HostMut`, callback captures, or `HostContext` state modified
through interior mutability. Embedders that need a fresh guest-state boundary pair the operation with
`reset_decision()` and keep authoritative state in host slots.

Successful `invoke` returns `InvocationOutcome { value, records, usage }`; its record slice borrows Rustia's
reusable journal until the next mutable use. `invoke_into` reserves the caller-owned record lane before guest
execution and makes the final append allocation-free and infallible, returning `InvocationReceipt` with the
value plus fuel/count/byte usage. See the compile-checked
[`typed_invocation.rs`](../crates/rustia/examples/typed_invocation.rs).

### 3. Serde-first data bridge

This replaces the existing koto scenario/config path.

```rust
#[derive(Deserialize)]
struct ScenarioSpec {
    enemies: Vec<EnemySpec>,
}

let exports = vm.compile_and_run(script)?;
let spec: ScenarioSpec = exports.deserialize(&mut vm)?;
```

Also support the reverse direction:

```rust
vm.insert_serde("agent", &agent_state)?;
let decision: Decision = value.deserialize()?;
let root = value.table().unwrap();
let enemies = root.get_string_table("enemies")?.unwrap();
let first_enemy: EnemySpec = enemies.get_array_deserialized(0)?.unwrap();
```

Start with owned-value serde for correctness and ergonomics; `OwnedValue` plus string-keyed and array-slot
`OwnedTable` deserialization helpers are the first public slice. Add the zero-copy live-value deserializer after
the surface is stable, matching the architecture note that zero-copy serde is faster by avoiding the deep-copy
exports snapshot.

Owned-value serde uses Serde's externally tagged enum model. A unit variant is a string, while a newtype,
tuple, or struct variant is a single-entry string-keyed table whose value carries the variant content. Derived
`rename` and `rename_all` rules apply to the string key exactly as they do for other Serde formats. Serialization
produces those shapes and deserialization accepts them symmetrically; other table shapes and non-string variant
keys fail explicitly.

### 4. Compiled module and hot-reload API

Hot reload is part of the embedding goal, not tooling polish.

```rust
let program = Rustia::compile("ai.luau", source)?;
vm.install(program)?;

match vm.hot_reload("ai.luau", new_source) {
    Ok(exports) => {
        // swap to new exports
    }
    Err(error) => {
        // keep old program/exports alive
    }
}
```

Requirements:

- compiling new source must not mutate the current VM state.
- source names and spans must survive into diagnostics.
- old program remains callable on reload failure.
- retained handles are versioned or explicitly invalidated across reload.

Current implementation slice: source-named run/compile helpers preserve host-provided names in runtime
diagnostics, and `rustia::Error` exposes public source-span/runtime-location accessors for host-side diagnostic
renderers. `Exports::generation()`, `Function::generation()`, and `Rustia::generation()` report the successful
install generation. `is_current()` / `ensure_current()` let hosts check retained handle freshness, and
`Function::call_current()` and `Exports::{value,deserialize,get,function}_current()` give hosts fail-closed
access paths for stale handles. Failed hot reloads do not advance the generation, and old retained function
handles remain callable through `Function::call()` because the VM retains prior dynamic programs.
That is not yet transactional hot reload: a new module that starts executing and then fails can leave partial
global/heap effects even though the generation does not advance, and extension installation can partially
register earlier functions before a later validation failure. Public release needs prevalidation plus
transactional/isolated failure semantics, or an explicit narrower contract with host-visible recovery.

### 5. Optional macro/codegen sugar

An early `rustia_extension!` declarative macro is implemented. The attribute-macro/codegen shape below is a
future concept and should wait until the lower-level API is stable.

```rust
#[rustia_module]
mod ai_host {
    #[pure(fuel = 2)]
    fn fixed_mul(a: i32, b: i32) -> i32 {
        ((a as i64 * b as i64) >> 16) as i32
    }

    #[effect(fuel = 5)]
    fn emit_move(ctx: &mut HostCtx, dx: i32, dy: i32) {
        ctx.effect(Move { dir: [dx, dy] });
    }
}
```

This can later grow into schema-driven binding generation for larger hosts, inspired by Wasmtime's component
model and Deno's extension/op system. Do not start here; macros freeze mistakes.

## Grabby Fit

The public API works for grabby with `HostModule<GrabbyHost>` as the persistent registration layer, a
`CompiledEntry` as the host-owned decision artifact, and a fresh borrowing `TickContext` as the invocation
authority. Setup constructs the hardened VM and compiles the direct decision body:

```rust
let mut vm = RustiaBuilder::deterministic_leaf()
    .memory_limit_bytes(memory_limit)
    .build_for_host::<GrabbyHost>()?;
assert_eq!(
    vm.execution_profile(),
    RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF,
);
vm.install_host_module(bridge)?;
let decide = Rustia::<GrabbyHost>::compile_entry("decide.luau", source)?;
```

The typed execution profile above is also available from the builder. Its
versioned canonical bytes and compact compatibility ID let Grabby bind the
actual selected `Profile` plus `GuestSurface` component into its package
identity without duplicating a string label. It does not encode settings,
budgets, services, dependency features, or host ABI; an embedder must bind
those separately.

The worker hot path is:

```rust
let mut context = TickContext {
    emitter,
    input: &agent_input,
    slots: &slot_snapshot,
};
let receipt = vm.invoke_entry_into::<Decision>(
    &mut context,
    InvocationLimits::new(fuel)
        .with_max_records(max_records)
        .with_max_record_bytes(max_record_bytes),
    &mut worker_lane,
    &decide,
)?;
```

The grabby bridge owns:

- current `emitter: SimId`.
- each tick/phase's block-local input and captured slot projections.
- conversion from typed effects/slot writes to the shared `EffectRecord`.
- local lane effect buffers.
- the rule that at most one invocation succeeds per `(tick, phase, emitter)`; a failed transaction may retry.
- stable-sort commit by `(emitter, seq)`.
- `SetSlot` writeback to captured columnar slot arrays.

rustia owns:

- one shared fuel budget for the full invocation and its nested calls.
- typed argument/result conversion.
- capability views and profile enforcement.
- bounded record count/logical bytes, one dense sequence across effects and slot writes, callback savepoints,
  and all-or-nothing outer record publication.
- usage receipts; `invoke_entry_into` reserves before execution, appends to the lane without allocating at
  commit, and resets after both success and failure.
- `reset_decision()` for per-decision `DeterministicSim` leaves, with decision-reset ABI v1, idle-only execution,
  sealed immutable-environment reuse, decision-local allocation discard, deterministic
  RNG/GC/logical-time restart, and epoch invalidation of every retained handle.
- the hardened guest-surface ABI, frozen allowlisted environment, mandatory memory limit, and rejection of
  guest globals/upvalues/raw access/dynamic authority.
- stateless compiled-entry execution without an exports table, retained closure, or loaded-program installation.
- install-generation/freshness checks plus VM/epoch-rooted handle ownership; transactional hot reload remains
  planned.

The hot AI leaf remains a per-decision pure function. Coroutines are still valid inside one decision and in
scenario/console paths, but a coroutine must not persist across ticks in DeterministicSim because that would
store authoritative state on the GC heap instead of in captured host slots.

## Build Order

1. ✅ `OwnedValue` plus `FromRustia` / `IntoRustia` and typed/nested `OwnedTable` helpers — first slice landed.
2. ✅ typed `add_fn` and `Function::call` — landed through tuple arity 8.
3. ✅ returned-table `Exports` facade — landed with generation-aware accessors.
4. ✅ owned serde deserialize/serialize — landed; rich graphs and live zero-copy values remain.
5. ✅ extension/linker with capability and fuel metadata — first slice landed.
6. ✅ hot reload and diagnostics — first slice landed; reclamation and stack traces remain.
7. ✅ early declarative macro sugar — landed; broader attribute/codegen expansion is deferred.
8. ✅ runtime output/time services — landed per [`72-host-services.md`](72-host-services.md): environment
   presets, `ServiceClock` provider sockets, real `print` composition, pinned-epoch and logical-clock
   fallbacks; honest host-facing VM-owned telemetry (`Vm::memory_stats()`) is now implemented by the later
   accounting tranche, while script-visible counters await the O(1) quota tracker.
9. ✅ logical-live quotas and atomic/catchable OOM — implemented through
   `VmSettings::memory_limit_bytes` / `RustiaBuilder::memory_limit_bytes`; exact-commit native Linode and wasm32
   pressure validation is retained under `safe-memory-588b541`, while O(1) accounting remains an optimization.
10. ✅ epoch-safe decision reset — `Vm::reset_decision()` / `Rustia::reset_decision()` preserve host
    configuration, discard all guest state, restart deterministic counters, invalidate every old retained
    handle clone, and plateau exactly in the 600-decision mixed lifecycle test.
11. ✅ typed host schema/context, capability views, typed modules, shared invocation fuel, bounded ordered
    records, handler savepoints, outer publication rollback, usage receipts, and host-panic poisoning/reset.
12. ✅ hardened stateless action leaves — deterministic allowlisted/frozen guest surface, mandatory memory
    limit, bytecode policy validation, direct `CompiledEntry` invocation, failure-safe lifecycle helper,
    immutable-prefix decision reset, and native/wasm32 exact-plateau probes.
13. ⏳ host-owned script module resolution, transactional install/reload of guest state, and rich/live values —
    next.

Defer mlua-style `UserData`/`Scope` until the above is stable. The first useful embedding target is typed
owned values, retained script function handles, serde, and deterministic extension registration.
