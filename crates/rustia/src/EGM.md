# rustia Facade Source

## Overview

This directory implements the application API. [lib.rs](./lib.rs) privately composes the modules and selectively re-exports public facade types; internal module files are not automatically public API.

## Interface

- `Rustia` and `RustiaBuilder`: construct and operate a VM through facade-owned settings and errors. The crate Rustdoc provides a task index, callback-state chooser, failure/reuse comparison, and owned/live/Serde value chooser; `Rustia` documents the thread-affine interpreter boundary.
- `Chunk`: named source, optional rooted lexical environment, then `eval`, `eval_live`, `eval_serde`, `exec`, or transactional `install`.
- `OwnedValue`/`IntoRustia`/`FromRustia`: owned data conversions; `Value` and `Table`: rooted live guest values.
- `Function`/`Exports` and `HostFunction`: retained script calls and reusable host callbacks.
- `HostSchema`, `HostContext`, `HostModule`, and `Invocation`: borrowed typed invocation context, capability-scoped callbacks, shared fuel, and bounded ordered output records.
- `ObjectType`, `Object`, and `NewObject`: typed Rust payload registration and guest method handles.
- `Profile`, `GuestSurface`, `Settings`, facade errors, runtime errors, and diagnostic types expose application policy and reporting.
- Other implementation modules: [extension_api.rs](./extension_api.rs) (builder/services and module-cohort cache contracts), [value.rs](./value.rs) and [serde_value.rs](./serde_value.rs) (conversions), [object_context.rs](./object_context.rs)/[object_method.rs](./object_method.rs) (typed method adapters), and [source_map.rs](./source_map.rs) (diagnostic mapping).
- Remaining source roles: [chunk.rs](./chunk.rs) (chunk builder and terminals), [invocation.rs](./invocation.rs) (typed host schema and call journal), [diagnostic.rs](./diagnostic.rs) and [diagnostic_data.rs](./diagnostic_data.rs) (bounded reports/snapshots, documented byte/line units and source omission), [error.rs](./error.rs), [runtime_error.rs](./runtime_error.rs), and [source_error.rs](./source_error.rs) (facade error types and runtime failure/recovery guidance), [settings.rs](./settings.rs) and [profile.rs](./profile.rs) (configuration), [function.rs](./function.rs) and [host_function.rs](./host_function.rs) (script/host function handles), [live_value.rs](./live_value.rs) (live handles), [object.rs](./object.rs), [object_conversion.rs](./object_conversion.rs), and [object_context.rs](./object_context.rs) (typed Rust objects), [macros.rs](./macros.rs) (declarative sugar), [brand_contract.rs](./brand_contract.rs) (doctest contract), and [source_map.rs](./source_map.rs) (source mapping). [engine_boundary_tests.rs](./engine_boundary_tests.rs) contains tests only and is excluded from the production interface.

## Invariants

- The facade does not expose raw VM `Value`, bytecode, heap identity, or GC lifetimes.
- Owned values are acyclic snapshots; live handles preserve identity and are bound to the creating VM and decision epoch.
- Reset requires an idle VM, preserves settings and persistent registrations, and invalidates prior live handles.
- `install` is guest-state transactional; `eval`/`exec` leave completed guest mutations in place on failure.
- Invocation callbacks only receive their declared capability views. Captured Rust state remains an embedder trust boundary.
- Deterministic invocation records have one dense sequence and bounded logical record bytes; host publication is all-or-nothing on escaping failure.

## Side Effects

Chunk execution mutates guest state and can call configured services or host callbacks. `install` stages guest changes for rollback. Host I/O, resolver access, and mutable Rust payload changes happen outside guest rollback. `reset_decision()` clears decision-local VM state and advances the epoch.

## Verification

From the repository root, run `cargo test --locked -p rustia` and `cargo test --locked -p rustia --doc`.
