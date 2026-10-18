# Koto to rustia migration guide

> **Status: first public migration slice.** This guide describes the current
> `rustia` embedding API. It is generic public guidance; grabby-specific host
> types and effect catalogs remain outside the public crate.

## Choose the embedding surface

Rustia intentionally has two surfaces:

- `Rustia<()>` is the convenient context-free/legacy facade. It supports direct
  `eval`/`exec`/`load`, `Function::call`, `add_fn`, and `RustiaExtension` use.
- `Rustia<H>` is the recommended surface for a simulation or other serious
  host. A lifetime-independent `HostSchema` describes authority, while every
  `invoke`/`invoke_into` borrows a fresh `HostContext<H>`. Guest execution is
  available only through the lexical `Invocation`.

For grabby-style work, start with the typed surface. It provides block-local
input/slot borrows, capability-specific callbacks, one shared fuel budget,
bounded transactional records, and allocation-safe lane publication.

## Concept map

| koto-side concept | recommended rustia replacement |
|---|---|
| engine/settings object | `Rustia::builder()`, per-limit methods, `Settings` readback, `.build_for_host::<H>()` |
| per-call execution context | `HostSchema` plus a freshly borrowed `HostContext<H>` |
| prelude/config insertion | `insert_global`, `insert_serde` |
| plain host callback | `add_pure`, `add_input`, `add_slot_read`, `add_effect`, `add_slot_write` |
| named serious-host API | `HostModule<H>` plus capability-specific builder methods |
| script module | `Rustia::compile_module`, then `Invocation::install` |
| retained script function | `Exports::function_in`, `Function::call_in`, VM/decision-epoch checks |
| execution timeout | `InvocationLimits::new(fuel)` |
| deterministic output | `Effects`/`SlotWrites`, `invoke_into`, caller-owned record lane |
| config/scenario tables | `OwnedValue`, typed conversion, and serde |
| simple context-free migration | `Rustia<()>`, direct `eval`/`load`/`Function::call`, `RustiaExtension` |

## Recommended typed port

### 1. Define authority, not one permanent context type

`HostSchema` contains only types and is `'static`. The actual context can carry
unrelated tick-local borrows:

```rust,ignore
struct SimHost;

impl HostSchema for SimHost {
    type Input = [AgentInput];
    type Slots = [i32];
    type External = ();
    type Effect = Effect;
    type SlotWrite = (usize, i32);
    type Record = EffectRecord;
}

struct TickContext<'input, 'slots> {
    input: &'input [AgentInput],
    slots: &'slots [i32],
    external: (),
    emitter: u32,
}

impl HostContext<SimHost> for TickContext<'_, '_> {
    fn input(&self) -> &[AgentInput] { self.input }
    fn slots(&self) -> &[i32] { self.slots }
    fn external_mut(&mut self) -> &mut () { &mut self.external }

    fn record_effect(&self, sequence: u32, effect: Effect) -> EffectRecord {
        EffectRecord::effect(self.emitter, sequence, effect)
    }

    fn record_slot_write(&self, sequence: u32, write: (usize, i32)) -> EffectRecord {
        EffectRecord::slot_write(self.emitter, sequence, write)
    }

    fn record_bytes(&self, record: &EffectRecord) -> u64 {
        record.logical_bytes()
    }
}
```

`record_bytes` is a deterministic logical charge, not allocator capacity.

### 2. Build a typed VM

```rust,ignore
let mut rustia = Rustia::builder()
    .profile(Profile::DeterministicSim)
    .fuel(100_000)
    .build_for_host::<SimHost>()?;
```

The builder preserves profile, VM settings, and services. Legacy
`RustiaExtension` values deliberately cannot cross into a typed VM; use
`HostModule<SimHost>`. Builder `.fuel(...)` is a VM-wide ceiling; each typed
invocation uses the lower of that ceiling and its explicit `InvocationLimits`
fuel.

### 3. Register the narrowest capabilities

```rust,ignore
let host = HostModule::<SimHost>::new("host")
    .pure("fixed_mul", 1, |a: i32, b: i32| {
        ((a as i64 * b as i64) >> 16) as i32
    })
    .input("agent_count", 1, |input: Input<'_, SimHost>| {
        input.get().len()
    })
    .slot_read("read_slot", 1, |slots: Slots<'_, SimHost>, slot: usize| {
        slots.get()[slot]
    })
    .effect(
        "emit",
        2,
        |mut effects: Effects<'_, SimHost>, effect: Effect| -> rustia::Result<()> {
            effects.emit(effect)
        },
    )
    .slot_write(
        "write_slot",
        2,
        |mut writes: SlotWrites<'_, SimHost>, slot: usize, value: i32|
            -> rustia::Result<()> {
            writes.write((slot, value))
        },
    );

rustia.install_host_module(host)?;
```

`DeterministicSim` rejects `external` and `nondeterministic` functions before
any part of a module becomes callable. Function metadata exposes names,
capabilities, fuel charges, argument/return types, and fallibility.

### 4. Compile once; execute inside an invocation

```rust,ignore
let module = Rustia::compile_module("ai.luau", source)?;

let installed = rustia.invoke(
    &mut initial_context,
    InvocationLimits::new(50_000),
    |invocation| invocation.install(&module),
)?;
let (exports, _, _) = installed.into_parts();
```

Source compilation itself needs no host context. Module execution does, so
installing or running it occurs inside `Invocation`.

### 5. Make the decision and publish records

```rust,ignore
let limits = InvocationLimits::new(20_000)
    .with_max_records(16)
    .with_max_record_bytes(2 * 1024);

let receipt = rustia.invoke_into(
    &mut tick_context,
    limits,
    &mut effect_lane,
    |invocation| {
        let decide = exports.function_in(invocation, "decide")?;
        let decision: Decision = decide.call_in(invocation, (agent_id,))?;
        Ok(decision)
    },
)?;
```

All guest calls in the closure share the same fuel and record budgets. A
successful callback appends to one dense effect/slot-write sequence. A failed
callback loses its suffix; an error escaping the invocation publishes nothing.
`invoke_into` reserves the caller lane before guest execution, so its final
append is allocation-free and infallible.

For grabby, allow at most one successful invocation for each
`(tick, phase, emitter)`. If one logical decision needs several guest calls,
put them in the same invocation. Rustia owns staging, dense ordering,
savepoints, and outer rollback; grabby owns record construction, lane merging,
world-application atomicity, and duplicate-decision prevention.

### 6. Handle retained values and serde

Use `Exports` and `Function` for retained script code and `OwnedValue` for a
detached data snapshot. On the typed surface, use `value_in`, `get_in`,
`function_in`, and `call_in`. Successful unrelated installations do not stale
retained handles; reset invalidates all retained handles by decision epoch.

```rust,ignore
let scenario: Scenario = rustia.invoke(
    &mut context,
    InvocationLimits::new(20_000),
    |invocation| {
        let owned = exports.value_in(invocation)?;
        owned.deserialize()
    },
)?.into_parts().0;
```

## Context-free compatibility port

Small tools that need no per-call authority or transactional outputs can keep
the simpler facade:

```rust,ignore
let mut rustia = Rustia::builder()
    .profile(Profile::ReproducibleStandard)
    .fuel(50_000)
    .build()?;

rustia.add_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))?;
let result: f64 = rustia
    .load("return clamp(5, 0, 3)")
    .source_name("tool.luau")
    .eval()?;
```

`RustiaExtension` remains the context-free namespaced module helper. Do not use
it as the simulation-authority boundary; it cannot receive a fresh
`HostContext`.

## Determinism and failure boundary

`DeterministicSim` accepts pure work, input reads, slot reads, deterministic
effect records, and slot-write records. It rejects external mutable-state
access and nondeterministic effects.

Fuel exhaustion, OOM, record overflow, invalid boundary yield, missing typed
dispatch, and host panic are hard typed-transaction aborts. A host panic
poisons the VM, and every later guest execution entry point rejects until
`reset_decision()` succeeds.

The general invocation transaction covers Rustia's record journal only. It does
not roll back guest heap/global mutation from ordinary execution, `HostMut`,
callback captures, or context mutation hidden behind interior mutability.
`Invocation::install` additionally wraps the complete candidate in a nested
guest-state savepoint and returns exports only after that savepoint commits.

## What not to port directly

- Wall-clock execution timeouts. Use deterministic instruction fuel.
- Unchecked references to internal script values. Use an acyclic `OwnedValue`
  snapshot or rooted VM/epoch-safe `Value`, `Table`, `Exports`, or `Function`
  handle.
- Arbitrary host mutation from deterministic scripts.
- One invocation per helper call when those calls form one logical decision.
- Public grabby types or domain effect enums in rustia itself.

## Current gaps

- Atomic candidate installation is implemented; external callback and resolver
  effects remain host-owned and cannot be rolled back.
- Rooted `Value`, `Table`, and `Exports` handles now deserialize directly through
  snapshot-free live-value serde. `OwnedValue` remains the explicit eager,
  acyclic validation and serialization path.
- Userdata/scope APIs remain deferred while the rich-value boundary evolves.
- Async and reentrant host callbacks are intentionally outside this first
  lexical invocation design.
- Public `0.1.0` stability still waits on packaging and the remaining
  correctness/performance gates.
