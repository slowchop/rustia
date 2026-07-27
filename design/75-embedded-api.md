# rustia - embedded API goal and plan

> **Status: [IMPLEMENTED FIRST SLICE - stabilization and advanced interop pending] (updated 2026-07-25).**
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
Object/program reclamation, real VM-owned live/reserved/peak telemetry, and the correctness-first logical-live
quota/atomic OOM path are now implemented and validated by the exact-commit `safe-memory-588b541` native Linode
cohort plus its wasm32 512 KiB quota/recovery probe. The next release-critical slice is not more facade
decoration: it is O(1) quota accounting, epoch-safe decision reset, a cheap worker-local/per-call host context,
a host-owned module resolver/cache,
transactional installation/reload failures, rich graph-preserving values, and live zero-copy serde.

## Non-Negotiable Requirements

- Koto-shaped migration path: construct VM, configure limits/fuel, register host functions, inject globals,
  compile/run chunks, read exports, call script functions from Rust.
- Luau-compatible script semantics remain the runtime contract.
- Public API must not leak gc-arena lifetimes or internal `Value` representation.
- Public rustia contains no `SimId`, `EffectRecord`, `ControlInput`, or grabby-specific enum.
- DeterministicSim allows deterministic host seams, not arbitrary host mutation.
- Cross-tick authoritative state for grabby lives in host typed slots, never Lua globals/upvalues.
- Host-effect output is ordered by the grabby bridge using `(emitter, seq)`, then committed by the host.
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
let bridge = RustiaExtension::new("grabby")
    .fn_pure("fixed_mul", fixed_mul)
    .fn_slot_read("slot", read_slot)
    .fn_slot_write("set_slot", enqueue_set_slot)
    .fn_effect("move", enqueue_move)
    .fn_effect("fire", enqueue_fire);

let mut vm = Rustia::builder()
    .profile(Profile::DeterministicSim)
    .extension(bridge)
    .build()?;
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

Current implementation slice: `Rustia::add_pure_fn`, `add_input_fn`, `add_slot_read_fn`, `add_slot_write_fn`,
`add_effect_fn`, `add_external_state_fn`, and `add_nondeterministic_fn` expose the same capability vocabulary on
the global facade. `RustiaExtension::fn_external_state` fills the remaining named extension helper.
`DeterministicSim` rejects external mutable-state reads and nondeterministic effects at registration time.
`RustiaExtension::namespace()` and `functions()` expose pre-install metadata for namespace, function name, fuel
ABI version, fuel cost, capability, parameter count, parameter Rust type names, return Rust type names, and
whether the callback is fallible. `RustiaExtension::fuel_policy()` exposes the host-function fuel ABI version,
default helper cost, and minimum accepted cost so hosts can snapshot their extension cost schedule.

Current limitation: callbacks are `Fn + Send + Sync + 'static` and receive typed script arguments but no
per-VM/per-call host context. Mutable worker-local slot/effect state therefore requires captured synchronization
or interior mutability today. Add an explicit context/state seam before treating the conceptual grabby sketch as
a cheap production bridge.

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

The public API works for grabby if grabby uses the extension/linker layer as the integration point. This is a
conceptual host-side sketch: `agent_input`, `slot_snapshot`, and effect-buffer ownership are grabby-side types,
not compile-checked public rustia API.

```rust
let ai = Rustia::compile("alien_ai.luau", source)?;
let exports = vm.install(ai)?;
let decide = exports.function(&mut vm, "decide")?;

decide.call(&mut vm, (agent_input, slot_snapshot))?;
let effects = bridge.take_effects();
```

The grabby bridge owns:

- current `emitter: SimId`.
- intra-agent `seq`.
- conversion between typed host functions and `EffectRecord`.
- local lane effect buffers.
- stable-sort commit by `(emitter, seq)`.
- `SetSlot` writeback to captured columnar slot arrays.

rustia owns:

- deterministic execution and fuel.
- typed argument/result conversion.
- capability enforcement.
- a future scratch-heap reset operation for per-decision DeterministicSim leaves; reconstruct the VM per
  decision until that operation and epoch-safe retained-handle invalidation land.
- current generation/freshness checks, plus planned true-root handle ownership and transactional hot reload.

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
10. ⏳ epoch-safe reset, per-call host context, modules, transactional install/reload, and rich/live values —
    next.

Defer mlua-style `UserData`/`Scope` until the above is stable. The first useful embedding target is typed
owned values, retained script function handles, serde, and deterministic extension registration.
