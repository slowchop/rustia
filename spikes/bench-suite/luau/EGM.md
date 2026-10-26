# luau

## Overview

Luau workload scripts consumed by the reference-Luau harness.

## Interface

- Workloads: [01_dense_tables.luau](./01_dense_tables.luau), [02_sparse_tables.luau](./02_sparse_tables.luau), [03_oop_metamethods.luau](./03_oop_metamethods.luau), [04_closures_upvalues.luau](./04_closures_upvalues.luau), [05_string_patterns.luau](./05_string_patterns.luau), [06_string_format.luau](./06_string_format.luau), [07_coroutines.luau](./07_coroutines.luau), [08_pcall_xpcall.luau](./08_pcall_xpcall.luau), [09_generic_iteration.luau](./09_generic_iteration.luau), [10_stdlib_math_table.luau](./10_stdlib_math_table.luau), and [11_serialization.luau](./11_serialization.luau).

## Invariants

- The harness expects each script to return a function of n that performs n units of work and yields a checksum.

## Side Effects

- The harness executes scripts repeatedly when measuring; do not run it locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/bench-suite/Cargo.toml` (static compile of the consuming harness; no timing run).
