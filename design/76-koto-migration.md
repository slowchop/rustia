# Koto to rustia migration guide

> **Status: first public migration slice.** This guide describes the `rustia`
> embedding API that exists today. It is generic public rustia guidance; actual
> grabby integration code, effect catalogs, and host types remain out of scope.

## Goal

Move an existing koto-style Rust embedding to rustia without depending on
internal VM values, grabby-specific public types, or nondeterministic host
state. The target public shape is:

- construct and configure a `Rustia` VM with an explicit profile and fuel
  budget.
- inject typed globals and serde data.
- register typed host functions with explicit deterministic capabilities.
- run named source chunks and read typed results.
- retain export/function handles safely across hot reload.
- keep all GC-owned script values behind owned snapshots or retained handles.

## Concept map

| koto-side concept | rustia replacement |
|---|---|
| engine/settings object | `Rustia::builder()`, `Profile`, `VmSettings`, `.fuel(...)` |
| prelude/global insertion | `insert_global`, `insert_serde` |
| plain Rust callback registration | `add_fn`, `add_fn_with`, capability-specific helpers |
| named host module | `RustiaExtension::new("name")` plus `.fn_*` helpers |
| script return value | `run<T>()`, `run_owned()`, `OwnedValue`, `FromRustia` |
| config/scenario tables | `serde` through `OwnedValue` / `Exports::deserialize` |
| retained script function | `Exports::function`, `Function::call`, generation checks |
| hot reload | `hot_reload`, `*_current` accessors, `is_current`/`ensure_current` |
| execution timeout | instruction-count fuel through `VmSettings` / builder `.fuel(...)` |

## First useful port

1. Choose the profile first.

   Use `Profile::ReproducibleStandard` for public deterministic-by-default
   behavior. Use `Profile::DeterministicSim` only when scripts must stay inside
   the stricter deterministic host boundary.

2. Replace VM construction with the builder.

   ```rust
   let mut rustia = Rustia::builder()
       .profile(Profile::DeterministicSim)
       .fuel(50_000)
       .build()?;
   ```

3. Preserve source names at every host/script boundary.

   Prefer `run_named`, `run_owned_named`, `compile_and_run_named`,
   `compile_and_run_exports_named`, and `hot_reload("name.luau", source)` over
   anonymous source entry points when diagnostics matter.

4. Move globals through typed conversion or serde.

   ```rust
   rustia.insert_global("difficulty", 3_i32)?;
   rustia.insert_serde("scenario", &scenario)?;
   ```

5. Register host functions with the narrowest capability.

   ```rust
   rustia.add_pure_fn("clamp", |x: f64, lo: f64, hi: f64| x.clamp(lo, hi))?;
   rustia.add_slot_read_fn("slot_value", || 4_i32)?;
   rustia.add_effect_fn("emit_score", |value: i32| value)?;
   ```

   Use `add_external_state_fn` or `add_nondeterministic_fn` only outside
   `DeterministicSim`; deterministic profiles reject those capabilities at
   registration.

6. Group serious host APIs with `RustiaExtension`.

   ```rust
   let extension = RustiaExtension::new("host")
       .fn_pure("fixed_mul", |a: i32, b: i32| ((a as i64 * b as i64) >> 16) as i32)
       .fn_slot_read("read_slot", || 5_i32)
       .fn_effect("emit", |value: i32| value);

   let mut rustia = Rustia::builder()
       .profile(Profile::DeterministicSim)
       .extension(extension)
       .build()?;
   ```

   Before installation, `RustiaExtension::functions()` exposes namespace,
   function name, host-fuel ABI version, fuel cost, capability, parameter
   types, return types, and fallibility. `fuel_policy()` exposes the current
   host-function fuel ABI policy so larger hosts can snapshot their cost
   schedule.

7. Convert script results through typed APIs first.

   ```rust
   let value: f64 = rustia.run_named("ai.luau", "return host.read_slot() + 1")?;
   let owned = rustia.run_owned_named("ai.luau", "return { kind = 'scout' }")?;
   let table = owned.table().unwrap();
   let kind = table.get_string_as::<String>("kind")?.unwrap();
   ```

8. Use serde for scenario/config data.

   ```rust
   let exports = rustia.compile_and_run_exports_named("scenario.luau", source)?;
   let scenario: Scenario = exports.deserialize(&mut rustia)?;
   ```

   Owned snapshots also support table-local serde:

   ```rust
   let root = exports.value(&mut rustia)?.table().unwrap();
   let enemies = root.get_string_table("enemies")?.unwrap();
   let first: Enemy = enemies.get_array_deserialized(0)?.unwrap();
   ```

9. Replace held koto values with retained handles or owned snapshots.

   Use `Exports` and `Function` when the host needs to call back into script
   code. Use `OwnedValue` when the host needs a detached data snapshot. Do not
   expose internal VM `Value` lifetimes outside rustia.

10. Make hot reload fail closed where required.

   `Function::call` intentionally keeps old retained functions callable after a
   successful reload. For strict current-program behavior, use
   `Function::call_current` and `Exports::{value,get,function,deserialize}_current`.

## Determinism boundary

`DeterministicSim` is a subset. It accepts pure functions, deterministic input
reads, slot reads/writes, and deterministic effects. It rejects external mutable
state reads and nondeterministic effects. Host code should keep authoritative
cross-tick state in host-owned typed slots, not Lua globals or upvalues.

For public rustia, effect queue ordering and domain-specific effect records are
host responsibilities. rustia provides capability checking, typed conversion,
instruction fuel, retained-handle generation checks, and source-name-aware
diagnostics.

## What not to port directly

- Wall-clock execution timeouts. Use instruction-count fuel instead.
- Long-lived references to internal script values. Use `OwnedValue`, `Exports`,
  or `Function`.
- Public grabby types such as `SimId`, effect enums, or slot schemas.
- Arbitrary host mutation from deterministic scripts.
- Coroutine state that persists across deterministic simulation ticks.

## Current gaps

- The first public serde path snapshots into `OwnedValue`; zero-copy live-value
  serde is still deferred.
- Userdata/scope APIs are intentionally deferred until the lower-level facade
  stops moving.
- The host-function fuel ABI is public, but internal instruction and
  superinstruction fuel schedule versioning still needs a broader runtime pass.
- Public `0.1.0` stability still waits on release packaging, benchmark history,
  and engine/performance gates.
