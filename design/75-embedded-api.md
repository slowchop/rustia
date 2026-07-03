# rustia - embedded API goal and plan

> **Status: [PLAN - build against this] (2026-07-03).** This records the public Rust embedding API direction
> after comparing rustia's constraints with current Rust scripting/runtime APIs. It refines the locked
> architecture axis H in [`30-architecture.md`](30-architecture.md) and the grabby bridge contract in
> [`70-host-bridge-contract.md`](70-host-bridge-contract.md).

## Goal Name

**Grabby-Compatible Embedded API**

This is the API goal, not just a feature label. It means rustia must provide a general-purpose, public Rust
embedding surface that is pleasant outside grabby, while also being strong enough for grabby's deterministic
per-agent AI loop without leaking grabby-specific types into public rustia.

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
    .build();

vm.add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))?;
vm.insert_global("difficulty", 3)?;

let exports = vm.compile_and_run("return { decide = function(x) return clamp(x, 0, 10) end }")?;
let decide = exports.function("decide")?;
let y: f64 = decide.call((12.0,))?;
```

Core pieces:

- `OwnedValue` as the public value firewall.
- `IntoRustia` / `FromRustia`.
- `IntoRustiaMulti` / `FromRustiaMulti`.
- typed host functions and retained script calls support tuple-shaped arguments/results through arity 8.
- retained handles for functions/tables.
- internal gc-arena `Value` remains behind the facade.

Reason: best fit for koto migration, grabby ergonomics, and public API clarity.

### 2. Extension/linker layer for serious hosts

This is the grabby integration point and the scalable public host-module API.

```rust
let bridge = RustiaExtension::new("grabby")
    .fn_pure("fixed_mul", fixed_mul)
    .fn_slot_read("slot", read_slot)
    .fn_slot_write("set_slot", enqueue_set_slot)
    .fn_effect("move", enqueue_move)
    .fn_effect("fire", enqueue_fire)
    .build();

let mut vm = Rustia::builder()
    .profile(Profile::DeterministicSim)
    .extension(bridge)
    .build();
```

Each registration carries:

- name and optional module namespace.
- typed signature / arity metadata.
- fuel cost.
- capability.
- profile policy.

The current coarse VM capability set should become more precise:

- `Pure`
- `ReadsDeterministicInput`
- `ReadsSlotState`
- `EmitsDeterministicEffect`
- `WritesSlotEffect`
- `ReadsExternalMutableState`
- `NondeterministicEffect`

`DeterministicSim` allows the first five and rejects the last two.

Current implementation slice: `Rustia::add_pure_fn`, `add_input_fn`, `add_slot_read_fn`, `add_slot_write_fn`,
`add_effect_fn`, `add_external_state_fn`, and `add_nondeterministic_fn` expose the same capability vocabulary on
the global facade. `RustiaExtension::fn_external_state` fills the remaining named extension helper. Deterministic
profiles reject external mutable-state reads and nondeterministic effects at registration time.

### 3. Serde-first data bridge

This replaces the existing koto scenario/config path.

```rust
#[derive(Deserialize)]
struct ScenarioSpec {
    enemies: Vec<EnemySpec>,
}

let exports = vm.compile_and_run(script)?;
let spec: ScenarioSpec = exports.deserialize()?;
```

Also support the reverse direction:

```rust
vm.insert_serde("agent", &agent_state)?;
let decision: Decision = value.deserialize()?;
```

Start with owned-value serde for correctness and ergonomics. Add the zero-copy live-value deserializer after the
surface is stable, matching the architecture note that zero-copy serde is faster by avoiding the deep-copy
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

Current implementation slice: `Exports::generation()`, `Function::generation()`, and `Rustia::generation()`
report the successful install generation. Failed hot reloads do not advance the generation, and old retained
function handles remain callable because the VM retains prior dynamic programs.

### 5. Optional macro/codegen sugar

Macros come after the lower-level API is stable.

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

The public API works for grabby if grabby uses the extension/linker layer as the integration point.

```rust
let ai = vm.compile("alien_ai.luau", source)?;
let decide = ai.exports().function("decide")?;

decide.call((agent_input, slot_snapshot))?;
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
- scratch-heap reset hooks for per-decision DeterministicSim leaves.
- function handle and hot-reload safety.

The hot AI leaf remains a per-decision pure function. Coroutines are still valid inside one decision and in
scenario/console paths, but a coroutine must not persist across ticks in DeterministicSim because that would
store authoritative state on the GC heap instead of in captured host slots.

## Build Order

1. `OwnedValue` plus `FromRustia` / `IntoRustia`.
2. typed `add_fn` and `Function::call`.
3. returned-table `Exports` facade.
4. serde deserialize/serialize.
5. extension/linker with capability and fuel metadata.
6. hot reload and diagnostics.
7. macro sugar.

Defer mlua-style `UserData`/`Scope` until the above is stable. The first useful embedding target is typed
owned values, retained script function handles, serde, and deterministic extension registration.
