# VM Execution Submodules

## Overview

This private module directory separates specialized VM call paths and host integration out of the main [vm.rs](../vm.rs) engine file.

## Interface

- [builtin_arguments.rs](./builtin_arguments.rs): argument shape checks for specialized builtins.
- [bytecode_calls.rs](./bytecode_calls.rs) and [expanded_calls.rs](./expanded_calls.rs): ordinary bytecode calls, expanded argument/result chains, and continuation handling.
- [host_functions.rs](./host_functions.rs): dispatch to facade-owned typed host callbacks.
- [host_objects.rs](./host_objects.rs): Rust payload type registration, construction, receiver validation, and method lookup.
- [require.rs](./require.rs): host-resolved module loading, coroutine-owned continuation, and cache publication.

## Invariants

- Call continuation state is explicit in VM frames so suspension/resumption does not rely on growing the native stack.
- A required module's zero/one result enters cache only after successful completion. Load ownership belongs to its coroutine/cohort and is released on failure, cancellation, reset, or abandoned coroutine collection.
- Host object receiver type is checked before callback argument conversion. Host payload state is outside guest savepoint rollback.
- Specialized builtin paths must preserve source evaluation order and fall back to ordinary calls when specialization shape does not apply.

## Side Effects

Call paths execute guest and host callbacks, charge fuel, and mutate VM state. `require` invokes a configured resolver and retains module programs while loading. Host method calls may perform immediate Rust-side effects according to their declared capabilities.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm` and `cargo test --locked -p rustia --test yieldable_require`.
