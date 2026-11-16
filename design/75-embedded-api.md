# rustia - embedded API goal and plan

> **Status: [COHERENT EXECUTION + SYNCHRONOUS HOST FUNCTIONS + TYPED RUST OBJECTS IMPLEMENTED; CONTROLLED-CLIENT ROLLOUT VERIFIED] (updated 2026-10-29).**
> This records the public Rust embedding API direction and its honest implementation boundary after comparing
> rustia's constraints with current Rust scripting/runtime APIs. It refines the locked
> architecture axis H in [`30-architecture.md`](30-architecture.md) and the grabby bridge contract in
> [`70-host-bridge-contract.md`](70-host-bridge-contract.md).

## Goal Name

**Grabby-Compatible Embedded API**

This is the API goal, not just a feature label. It means rustia must provide a general-purpose, public Rust
embedding surface that is pleasant outside grabby, while also being strong enough for grabby's deterministic
per-agent AI loop without leaking grabby-specific types into public rustia. Deterministic simulation is one
first-class builder contract, not a global restriction: ordinary applications can choose an explicitly live,
fully non-reproducible environment at construction time.

## Current boundary

The composable `load(...).source_name(...).environment(...)` execution API, owned scalar and table values,
typed host functions, extension metadata/capabilities, VM/epoch-safe `Function`/`Exports` handles, owned serde,
transactional installation, diagnostics,
early declarative macro sugar, facade-owned settings/errors, and the runtime host-services slice (environment presets, stdout,
determinism-labeled clock/entropy providers, and native/WASI/browser adapters per
[`72-host-services.md`](72-host-services.md)) are implemented.
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
cohort plus its wasm32 512 KiB quota/recovery probe. The epoch-safe decision reset, host-owned `require`
resolver, transactional installation, and reclaimable synchronous host-function construction are implemented
([78-host-functions.md](78-host-functions.md)). The
graph-preserving advanced spine plus its first ergonomic constructors are now implemented:
VM/epoch-safe rooted `Value`/`Table`/`Function` handles preserve binary strings, buffers, vectors, rich keys,
shared/cyclic table identity, mutation, raw equality, and ordinary or typed invocation exchange without
exposing collector lifetimes. `globals()` exposes the real mutable globals table, `create_table()` admits an
empty rooted table under the logical-live quota, and `Table::set_live()` bridges owned keys to identity-bearing
live values. `create_function`, persistent `register_function`, and `HostModule::function` share a
`HostFunction` definition, capability/fuel metadata, and typed owned/live argument and tuple-result conversions.
Heap functions own their callbacks; guest aliases keep replaced functions callable, collection reclaims
unreachable callbacks, and reset releases disposable captures while restoring persistent registrations.
Typed Rust-object construction and methods are implemented through `ObjectType<T, H>`,
`register_object_type`, `create_object`, and VM/epoch-bound `Object<T>` handles. Objects reuse the same
conversions and borrowed capability views; `NewObject<T>` constructs callback results. The
[private acceptance record](93-private-api-acceptance.md) records the accepted source, checks,
reviews, and real-consumer rollout, including civfun Player integration. The
[object contract](83-rust-objects.md) records lifetime, mutation, validation, and quota behavior.
The composable `load(source).source_name(...).environment(...)` builder provides borrowed
text/byte source, diagnostic names, exact rooted lexical environments, `eval`, `eval_live`, `eval_serde`,
`exec`, and `install` terminals,
side-effect-only execution; custom environments are validated before compilation, nested functions capture
the selected table, and hardened leaves reject dynamic loading before compilation or VM allocation.
Hardened leaves reject both table constructor authorities before allocation. `OwnedValue` remains the small
acyclic snapshot layer. This advanced layer is additive, not a replacement for the existing typed facade.
The ordered expansion is recorded in
[Build Order](#build-order).

### Lower-level transferable consumer slice (2026-11-16)

Issue #50 adds a deliberately lower-level integration surface used by Luvy.
`rustia-vm` keeps local `Rc`/`RefCell` ownership by default and offers an opt-in
`send` feature that selects transferable owners, service guards, retained roots
and `Send + Sync` Rust payloads. This lets an idle, exclusively owned VM move
between scheduler workers; it neither permits simultaneous calls nor makes the
application facade's handles transferable.

The compiler records one optional lexical name per upvalue descriptor. Rooted
VM primitives can inspect script function metadata, capture storage and exact
environments, replace captures while idle, traverse a table one raw pair at a
time, and control trusted readonly/metatable state. `retain_program_entry`
constructs a callable main function in an exact rooted environment without
executing guest code. These operations preserve VM/epoch checks, alias/cell
identity, guest savepoints, collection and logical-live admission.

`VmHostCall` is the borrowed re-entry boundary for trusted dispatchers. A nested
call uses the same `VmExecution`, fuel, recursion depth and dispatcher; it does
not store a borrowed host pointer in the VM. The saved-caller driver leaves the
large bytecode frame before synchronous host dispatch, roots all transferred
state, and restores source diagnostics and continuations afterward. This is the
consumer-proven slice, not a claim that the public facade now has mlua-style
`Scope`, async callbacks or unrestricted deep re-entry. Those ergonomic and
acceptance decisions remain open under issue #50.

Saved native completion frames are synchronous boundaries, not resumable Rust
builtin frames. Their callbacks report `coroutine.isyieldable() == false` and an
attempted yield raises the ordinary pinned Luau boundary error immediately, so
an inner protected call can catch it. Borrowed/direct native callback execution
uses the same boundary. Embedders that need suspension must use the explicit
guest coroutine resume/yield path rather than yielding through a synchronous
native callback.

This restriction is coroutine-local. A synchronous callback may resume a child
coroutine, and that child retains its own normal yieldability; Rustia restores
the parent's non-yieldable boundary when control returns.

## Application API boundary

The `rustia` facade owns its profiles, capabilities, settings, memory samples,
source/compile/runtime errors, and diagnostic data. These public types expose
no bytecode, raw heap identity, compiler hooks, conformance flags, or collector
schedule. Lower-level engine crates retain those controls under their own 0.x
contracts. There are no public `From` conversions from engine failures into
facade errors.

Construct ordinary and typed interpreters through `RustiaBuilder`;
`Rustia::default()` uses the same standard preset as `Rustia::builder()`.
Use `without_stdout()` for a silent VM, and configure fuel, recursion, memory,
reporting budgets, and providers through individual methods. The recursion
limit defaults to 512 and larger requests clamp to 512. Hardened leaves reject
output, clock, and entropy providers before constructing the VM.

`builder.settings()` and `vm.settings()` return immutable `Settings` snapshots.
Public getters describe effective limits and service declarations without
calling providers. `identity_bytes()` is an opaque, versioned encoding of every
effective setting, including internal options. It uses fixed-width integers and
explicit tags so pointer width and Rust enum layout do not affect it. A host
must still bind code/build identities, callback/provider implementations and
captured state, and resolver results separately. Memory samples expose getters;
tracking-disabled peaks remain `None`.

Profiles, guest surfaces, capabilities, runtime error categories, source error
categories, and diagnostic classifications are non-exhaustive. Match with a
fallback arm. Use `error.as_vm_error().map(|e| e.kind())` for runtime categories.
Callback adapter traits are sealed marker bounds; application-owned conversions
implement `IntoRustia`/`FromRustia`, and borrowed host contexts implement
`HostSchema`/`HostContext`. Engine callback factories and raw conversion methods
are private, including when a caller uses a generic adapter bound.

A captured non-string guest failure has no transferable guest identity in a
Rust `Error`. Returning that error from a Rust callback raises its original
formatted text as a string, even after reset or in another VM. Guest-only
`pcall`/`xpcall` preserve the original error object. Diagnostic snapshots keep
their existing shared reporting-budget lease; wrapping an error does not copy
or release that lease. External typed Rust causes remain downcastable.

External contract tests cover settings readback/identity, error ownership,
callback bounds and custom conversions. Private unit tests preserve the
collector and conformance control cases that applications no longer configure.
The facade boundary merged in [PR #63](https://github.com/slowchop/rustia-historic/pull/63)
as `8f8558ab`. WOTS, civfun, and Grabby completed the common-candidate rollout recorded in
[the private acceptance record](93-private-api-acceptance.md). The eventual 1.0 freeze
and wider public-release gates remain open.

## Non-Negotiable Requirements

Human-facing diagnostics use the shared [`Error::report()` contract](79-script-diagnostics.md):
bounded immutable executed source and logical callers, honest spans, typed host
causes and serde paths. The error is now an object; migrate old `Error::Vm(e)`
matches to `error.as_vm_error()` or `error.kind()`, constructors to `Error::from`
and `Error::embed`, and module resolvers to `rustia::Result<ModuleSource>`.
Clients render at existing reporting boundaries, not inside VM execution.
Console-generated prefixes are mapped with `SourceMap`, not guessed from the
latest input buffer. Guest strings and hardened-leaf failure classification
remain separate from host presentation.

- Koto-shaped migration path: construct VM, configure limits/fuel, register host functions, inject globals,
  compile/run chunks, read exports, call script functions from Rust.
- Two additive public layers: preserve the concise typed command/data facade, then add an advanced live-object
  layer for embedders that need identity, mutation, coroutines, userdata, and richer composition.
- Luau-compatible script semantics remain the runtime contract.
- Public API must not leak gc-arena lifetimes or internal `Value` representation.
- Public rustia contains no `SimId`, `EffectRecord`, `ControlInput`, or grabby-specific enum.
- DeterministicSim allows deterministic host seams, not arbitrary host mutation.
- `RustiaBuilder::live()` is a first-class intentionally nondeterministic choice, not an unsupported escape
  hatch. `standard()` keeps reproducible defaults while permitting explicit nondeterministic providers and
  capabilities. Advanced interop must remain available in every preset; the selected preset changes policy
  enforcement and defaults, not which general-purpose API layer exists.
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

## Public API End State: Two Additive Layers

Rustia should not force every embedder to pay the conceptual cost of a full live object model, and it should
not force advanced embedders to treat the VM as a sealed calculator. The target is two additive layers over
one engine:

1. **Typed command/data facade (existing and retained).** `insert_global`, `add_fn`, `eval<T>`, `Exports`,
   owned serde, extensions, fuel/capabilities, and transactional installation remain the shortest route for deterministic
   decisions, configuration, and simple applications.
2. **Advanced live-object API (spine implemented; ergonomics/integration continuing).** Identity-safe live
   `Value` and `Table` handles now share the rooted VM/epoch model with `Function`; `eval_live`, live table
   lookup/mutation/iteration, live function arguments/results, typed-invocation exchange, binary
   string/buffer/vector access, rich keys, raw identity equality, live globals, quota-aware table construction,
   and owned-key/live-value insertion are implemented. A public coroutine `Thread` handle remains conditional
   on a real host-side scheduling caller. Reclaimable synchronous host-function construction and the
   composable chunk builder are implemented. Typed Rust-object methods and consumer use are verified
   in [the private acceptance record](93-private-api-acceptance.md); custom fields/metamethods, `Scope`, and async remain later layers.

The advanced layer is not a raw Luau C stack API and must not expose internal GC lifetimes. Handles belong to
one VM, are real GC roots while alive, fail safely when used with another VM, and carry an epoch contract:
ordinary handles survive successful module replacement, while explicit decision reset invalidates them.
Live tables preserve identity, shared/cyclic graphs, binary data, and mutation; `OwnedValue` remains the
portable deep-copy boundary for callers that prefer isolation.

Retained-handle misuse has a typed host contract: stale handles return
`VmErrorKind::StaleRetainedValue` with their retained and current decision epochs;
foreign handles return `VmErrorKind::RetainedValueFromDifferentVm`. Ownership is
checked before the decision epoch, so a foreign handle is identified correctly even
when the two VMs have different epochs. Inspect these through `Error::as_vm_error()`
rather than parsing text. Table access, retained-function calls, and export access
validate their receiver before running owned-value conversions, interning names, or
allocating guest keys and values. This applies to ordinary calls and typed invocations,
with owned or live arguments/results. A rejected receiver therefore cannot run caller
conversion code or consume the target VM's guest-memory or invocation-fuel budget.

The environment is chosen independently of the API layer:

| builder | promise | advanced API policy |
|---|---|---|
| `deterministic()` | strict lockstep/replay contract | Same live-object surface, but nondeterministic providers and host capabilities are rejected. Reset/epoch and fuel rules are mechanically enforced. |
| `standard()` | reproducible defaults | Same surface; callers may explicitly install external-state or nondeterministic functions and clocks, making that VM intentionally non-reproducible. |
| `live()` | ordinary, intentionally non-reproducible embedding | Same surface with system services and runtime compilation enabled by default. No replay guarantee is claimed; normal safety, memory, and Luau-semantic contracts still apply. |

This is an API-availability rule, not a promise that every feature is meaningful in every profile. For example,
async host work and suspended cross-tick coroutines are unsuitable for `DeterministicSim`, but they must not be
globally omitted merely because that profile exists.

### Typed Rust-object methods

`ObjectType<T, H>` registers a Rust payload type and its methods; `Object<T>` retains an instance.
The implemented model has two uses of the same API, following the existing host-function capability
rules. The [object guide](83-rust-objects.md) provides a short example and the detailed contract.

- **Deterministic game logic:** expose read-only methods over replay-stable input. Authoritative changes go
  through the existing typed effects or slot-write journal. Method callbacks can receive the matching typed
  capability view to stage those changes; never capture a separate mutable queue as a pure method.
  A successful invocation returns those requests
  for the host to apply; a failure escaping the invocation publishes none. A failing method discards its
  own staged suffix; earlier completed requests remain if guest `pcall` catches the error. Read-only access alone does not
  prove replay stability: captured state remains an embedding trust boundary.
- **Standard/live applications:** permit methods that mutate Rust-owned state or perform external work when
  explicitly declared with the corresponding host capability. External application changes and I/O happen
  immediately and remain after a later script error, failed installation, or decision reset. Rustia does not
  undo them. Reset still releases disposable instance payloads and invalidates their handles.

The complete method registration is validated against the selected profile before installation. A deterministic
VM rejects a definition containing forbidden capabilities atomically and preserves the previous definition.
Methods reuse synchronous host functions' argument/result conversions,
fuel, panic containment, capture bounds, and ownership rules.
Object handles remain VM/epoch-bound, preserve identity, and reject wrong receiver types safely.
Hardened leaves reject object construction before guest allocation.

Object lifetime accounts for host and guest aliases, replacement, collection, and reset. Replacing a definition
affects future instances; existing objects and detached methods retain their original methods. Reset releases
payloads even if stale Rust handles remain, and restores only current definitions. Host-owned Rust
payloads and their allocations have the same quota boundary as callback captures; VM-owned object storage
and roots receive atomic logical-live quota admission. Callback returns validate every fresh-object registration
and live handle before allocating guest results. Destructors must not panic and are not guest
finalizers. Persistent definitions and disposable instances follow the same explicit reset distinction
as host-function definitions and values. Payload references stay inside synchronous callbacks; they cannot
escape or re-enter the VM.

VM-owned mutable Rust objects with automatic snapshots or rollback are outside this milestone. So are
custom fields/metamethods, borrowed scopes, and async methods. A real consumer must exercise the minimum
surface before its API is considered settled.

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

let quick: f64 = vm.eval("return clamp(difficulty + 9, 0, 10)")?;
let exports = vm
    .load("return { decide = function(x) return clamp(x, 0, 10) end }")
    .source_name("ai.luau")
    .install()?;
let decide = exports.function(&mut vm, "decide")?;
let y: f64 = decide.call(&mut vm, (12.0,))?;
```

Core pieces:

- typed `Rustia::eval<T>()` for concise single-result conversion.
- `Rustia::load(...).source_name(...)` for host-controlled diagnostics and the
  `require` resolver referrer across every terminal operation.
- public error accessors for compile-time byte spans and runtime source/line locations.
- `OwnedValue` as the public value firewall.
- `IntoRustia` / `FromRustia`.
- `IntoRustiaMulti` / `FromRustiaMulti`.
- owned conversions for `Option<T>`, `Vec<T>`, deterministic `BTreeMap<String, T>`, direct `OwnedTable`
  values, typed `OwnedTable` helper constructors/getters, and nested table/array access.
- typed host functions support fallible `rustia::Result<T>` returns.
- typed host functions and retained script calls support tuple-shaped arguments/results through arity 8.
- retained `Function` and `Exports` handles checked by VM identity and decision epoch.
- internal gc-arena `Value` remains behind the facade.

Runtime services are part of the same builder (implemented per [`72-host-services.md`](72-host-services.md)):
`RustiaBuilder::standard()` (= `Rustia::builder()`) adds a real stdout print sink on native targets while
keeping deterministic clock defaults; `deterministic()` is the bare `DeterministicSim` world; `live()` is
"normal Luau" with host stdout, system wall/monotonic clocks, an ambient initial random seed, and the stock
`loadstring` compiler. Native/WASI and default browser-Wasm platform adapters make that one-word preset work
without plumbing; bare Wasm disables the adapter and supplies the input sockets explicitly. The
`.stdout(...)`/`.wall_clock(...)`/`.monotonic_clock(...)`/`.entropy(...)` sockets take a sink closure,
`ServiceClock`, or `ServiceEntropy`; `system()`/`monotonic()`/`from_nondeterministic_fn(...)` are live, while
`fixed(...)`/`from_fn(...)` are replay-stable. With no provider plugged, no-argument `os.time`/`os.date` read a
pinned epoch, `os.clock()` reads a deterministic logical clock (executed instructions / 1_000_000), and
`math.random` begins at seed zero. On `DeterministicSim`, `build()` rejects every provider labeled
nondeterministic.

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
three limits and creates one monotonic budget shared by every `Invocation::install`, invocation chunk
execution, `Function::call_in`, nested guest/metamethod call, and host dispatch performed by the outer Rust closure. The
builder initial fuel, when configured, is a VM-wide ceiling; the effective typed budget is the
lower of that ceiling and `InvocationLimits::fuel()`. The
output bounds are checked before a record enters the journal; effects and slot writes share a single dense
sequence starting at zero. `HostContext::record_bytes` defines the deterministic logical payload charge.

There are two record checkpoints. Each host handler rolls back only the suffix it appended when it returns an
error or panics. The outer `invoke`/`invoke_into` rolls back every record when its operation returns an error,
including a guest failure or Rust result-conversion failure. Guest protection follows normal `pcall` structure:
a completed host call survives a later guest error caught by `pcall`; a failing handler's own partial suffix
does not.

Fuel exhaustion, configured recursion-limit exhaustion, OOM, record-count or record-byte overflow, an invalid boundary yield, a missing typed-host
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

This is intentionally a **record transaction**, not a general heap/context transaction. It does not undo guest
globals/tables, mutation through profile-gated `HostMut`, callback captures, or `HostContext` state modified
through interior mutability. `Invocation::install` is the deliberate exception: it adds a nested guest-state
savepoint for the module candidate while keeping external host effects outside rollback. Embedders that need a
fresh boundary for other guest operations pair them with `reset_decision()` and keep authoritative state in
host slots.

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

let spec: ScenarioSpec = vm
    .load(script)
    .source_name("scenario.luau")
    .eval_serde()?;
```

Also support the reverse direction:

```rust
vm.insert_serde("agent", &agent_state)?;
let decision: Decision = value.deserialize()?;
let root = value.table().unwrap();
let enemies = root.get_string_table("enemies")?.unwrap();
let first_enemy: EnemySpec = enemies.get_array_deserialized(0)?.unwrap();
```

Owned-value serde remains the explicit eager acyclic validation and serialization path. Rooted `Value`,
`Table`, and `Exports` handles now also provide snapshot-free deserialization directly over VM values, avoiding
the recursive `OwnedValue` graph while retaining `DeserializeOwned` so collector-backed borrows cannot escape.
This is not allocation-free: destination strings and collections still own their storage.

Owned-value serde uses Serde's externally tagged enum model. A unit variant is a string, while a newtype,
tuple, or struct variant is a single-entry string-keyed table whose value carries the variant content. Derived
`rename` and `rename_all` rules apply to the string key exactly as they do for other Serde formats. Serialization
produces those shapes and deserialization accepts them symmetrically; other table shapes and non-string variant
keys fail explicitly.

### 4. Compiled module and transactional replacement API

Hot reload is ordinary host orchestration over one atomic installation primitive. Rustia does not own a named
active-module registry: the host already owns the reference it wants to replace.

```rust
let program = Rustia::compile_module("ai.luau", source)?;
let mut active = vm.install(&program)?;

// Compile and run a complete candidate privately. Assignment is the swap.
if let Ok(candidate) = vm
    .load(new_source)
    .source_name("ai.luau")
    .install()
{
    active = candidate;
}
```

Requirements:

- compiling new source must not mutate the current VM state.
- source names and spans must survive into diagnostics.
- compile, runtime, quota, cache-admission, and result-retention failure must roll back guest state.
- old exports and functions remain callable after both failed and successful replacement until decision reset.
- every installation receives a fresh lazy `require` cache cohort selected by executing function origin.

Current implementation: `install()` runs the candidate in a nested guest-state savepoint. By default it creates
a fresh environment whose `_G` is itself and whose metatable inherits VM globals; `.environment(&table)` selects
an exact shared environment when the application deliberately wants one. Failure restores tables, metatables,
upvalues, buffers, coroutines, RNG, logical time, GC scheduling, module programs/caches, globals, and provisional
external roots. The complete rollback snapshot is charged to the VM memory quota and must fit before any
candidate mutation. Transaction-time collection may reclaim newly created unreachable values while retaining
the pre-savepoint graph needed for rollback; the one exception is the initial hardened-leaf installation,
which preserves its append-only allocation prefix until the environment is sealed. Successful weak-table
collection remains observable. Host callbacks and resolver I/O are
external effects and cannot be undone; typed record output stays provisional until its enclosing invocation
commits. Required modules share the candidate environment and a fresh lazy cohort. Old functions keep using
their originating cohort, and collector reachability reclaims cohorts when no strong guest or host path can
reach them. `RUSTIA_INSTALL_TRANSACTION_ABI_VERSION` and `RUSTIA_MODULE_RESOLUTION_ABI_VERSION` version these
contracts.

#### Yielding script modules

Module-resolution ABI v3 lets module code yield inside a guest coroutine. The
loading continuation retains its code, environment, and pending values until
the caller resumes it. The cache publishes only after successful completion;
zero or one return value is allowed. Failures and multiple-result modules are
not cached. Resolution still runs before every cache lookup, so a host may
cache expensive source retrieval separately.

Loading ownership belongs to the executing coroutine and cache cohort. A nested
request for that owner's active identity reports a cycle. A different coroutine
requesting the same in-progress identity gets an error; it cannot take over or
observe a provisional result. Other identities and coroutines may run normally.
Error unwinding, close, collection of an abandoned coroutine, hard abort, and
decision reset release the corresponding loading entries. Loading-owner IDs do
not keep otherwise unreachable coroutines alive.

Ordinary `require` has ordinary call failure semantics: completed guest writes
remain, and completed typed records remain if the error is caught. It does not
hold a whole-VM savepoint across suspension. An escaping invocation error still
discards its provisional records. An enclosing failed `install` restores the
guest/cache/coroutine snapshot and typed savepoint, including loaders that were
already suspended before installation. Host effects remain outside rollback.
Fuel is shared with the caller; module loading does not create a fresh budget.

`require` is host policy rather than part of the standalone Luau language oracle
([target boundary](05-luau-target.md)). The pinned upstream
[loading continuation](https://github.com/luau-lang/luau/blob/0.709/Require/src/RequireImpl.cpp#L130)
commits a result after loading completes; Rustia specifies its own cohort,
zero-result caching, ownership, and failure rules above. The acceptance tests in
`crates/rustia/tests/yieldable_require.rs` cover nested/repeated suspension,
completion, interleaving, failure/retry, cycles, cancellation, collection, reset,
installation rollback, fuel abort, and typed-object roots on native and Node/V8.

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
let decide = Rustia::compile_entry("decide.luau", source)?;
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
- VM/epoch-rooted handle ownership plus atomic candidate installation and host-owned replacement.

The hot AI leaf remains a per-decision pure function. Coroutines are still valid inside one decision and in
scenario/console paths, but a coroutine must not persist across ticks in DeterministicSim because that would
store authoritative state on the GC heap instead of in captured host slots.

## Build Order

1. ✅ `OwnedValue` plus `FromRustia` / `IntoRustia` and typed/nested `OwnedTable` helpers — first slice landed.
2. ✅ typed `add_fn` and `Function::call` — landed through tuple arity 8.
3. ✅ returned-table `Exports` facade — landed with VM/epoch-safe rooted accessors.
4. ✅ owned serde deserialize/serialize plus snapshot-free live-value deserialization — landed.
5. ✅ extension/linker with capability and fuel metadata — first slice landed.
6. host-owned hot replacement and diagnostics — atomic installation, reclamation, and bounded logical call traces landed.
7. ✅ early declarative macro sugar — landed; broader attribute/codegen expansion is deferred.
8. ✅ runtime output/time services — landed per [`72-host-services.md`](72-host-services.md): environment
   presets, `ServiceClock`/`ServiceEntropy` provider sockets, real `print` composition, pinned-epoch and
   logical-clock fallbacks, and native/WASI/browser adapters; honest host-facing VM-owned telemetry
   (`Vm::memory_stats()`) is implemented, and script-visible counters read the maintained logical-live total.
9. ✅ logical-live quotas and atomic/catchable OOM — implemented through
   `RustiaBuilder::memory_limit_bytes`; exact-commit native Linode and wasm32
   pressure validation is retained under `safe-memory-588b541`; maintained heap/program/control totals now
   remove whole-VM census scans from quota-mode instruction boundaries. The shared native/Node-V8 pressure test
   adds 96 epochs and 3,072 mixed allocation waves under a 2 MiB quota, including atomic 8 MiB allocation
   failures and recovery.
10. ✅ epoch-safe decision reset — `Vm::reset_decision()` / `Rustia::reset_decision()` preserve host
    configuration, discard all guest state, restart deterministic counters, invalidate every old retained
    handle clone, and plateau exactly in the 600-decision mixed lifecycle test.
11. ✅ typed host schema/context, capability views, typed modules, shared invocation fuel, bounded ordered
    records, handler savepoints, outer publication rollback, usage receipts, and host-panic poisoning/reset.
12. ✅ hardened stateless action leaves — deterministic allowlisted/frozen guest surface, mandatory memory
    limit, bytecode policy validation, direct `CompiledEntry` invocation, failure-safe lifecycle helper,
    immutable-prefix decision reset, and native/wasm32 exact-plateau probes.
13. ✅ **Add the advanced live-handle spine:** identity-safe live `Value` and `Table` handles now share the
    existing rooted `Function` VM/epoch model. `eval_live`, live lookup/mutation/iteration, live function
    arguments/results, typed-invocation exchange, binary strings, buffers, vectors, rich keys, userdata
    identity, raw equality, and shared/cyclic table graphs do not expose GC lifetimes. A public `Thread` handle
    remains intentionally absent until a concrete host-side scheduling caller defines its contract.
14. ✅ **Add the familiar ergonomic entry points:** `globals()`, quota-aware `create_table()`,
    owned-key/live-value insertion, and the composable
    `load(source).source_name(...).environment(...)` chunk builder is implemented. The builder borrows
    text or bytes, preserves named diagnostics, supports `eval`/`eval_live`/`eval_serde`/`exec`/`install`, executes against globals
    or an exact rooted lexical environment, and rejects invalid environment handles before compilation.
    `create_function()` now admits reclaimable callbacks as rooted values; `HostFunction` definitions share
    typed owned/live conversions and capability/fuel rules with `register_function` and `HostModule::function`.
    Reset invalidates disposable handles and releases their captures; persistent replacement releases
    obsolete configuration while preserving guest aliases. Keep `eval<T>()` and `add_fn` as concise sugar.
15. ✅ **Add host-owned modules:** script-visible `require`, deterministic module identity where promised,
    execute-once caching, cycle diagnostics, shared caller fuel, reset-aware cache lifetime, and
    host-controlled resolution rather than an ambient core filesystem API. The behavior is versioned by
    `RUSTIA_MODULE_RESOLUTION_ABI_VERSION`; the host must separately bind its resolver content.
16. **Complete rich integration and failure semantics:** transactional extension/module/replacement
    installation, explicit replacement/reset lifetimes, source diagnostics, and resumable
    `require` are implemented. The accepted candidate, correctness checks, independent reviews,
    and private-platform evidence are recorded in [93](93-private-api-acceptance.md).
17. **Validate the implemented typed Rust-object methods in a real consumer.** Registration, live
    handles, method dispatch, read-only replay-stable input plus staged actions, and explicit
    direct mutation in standard/live applications are implemented. Checks, independent reviews,
    and the merged civfun Player integration are recorded in [93](93-private-api-acceptance.md).
    Follow the [object contract](#typed-rust-object-methods) and the canonical
    [milestone](92-first-release.md). A full mlua-style object system, fields/metamethods, borrowed `Scope`,
    and VM-owned mutable-object snapshots remain optional later work.
18. ⏳ **Make async an optional advanced layer, not a core assumption:** Rust futures may bridge to Luau
    coroutines for tools and live applications; `DeterministicSim` may reject or narrowly constrain operations
    that cannot satisfy its replay contract.
19. ⏳ **Consider an mlua migration/compatibility layer after the native advanced API stabilizes.** Familiar
    names and call shapes may be provided additively, but do not claim import-swap compatibility without a
    dedicated compatibility suite, and do not let a shim erase Rustia's builder, capability, fuel, or reset
    contracts.

Steps 13–17 are the advanced embedding track; the minimal object methods in step 17 are required for the
API and consumer-readiness milestone. The larger object features named there and steps 18–19 remain
optional later layers. Every step applies to the shared engine; `live()` users are not forced through
`DeterministicSim` restrictions.
