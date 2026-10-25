# src

## Overview

Minimal fixed-width register bytecode dispatch experiment.

## Interface

- [lib.rs](./lib.rs) exports `bench`, `bench_threaded`, `last_fuel`, and `dispatches_per_run` for the Wasm runner.
- [main.rs](./main.rs) is the native timing reporter.

## Invariants

- Match and function-pointer variants execute the same fixed program.
- Instruction fuel is recorded in a mutable static for the single-thread experiment.

## Side Effects

- Benchmark exports execute many iterations; native binary measures elapsed time.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-b-dispatch/Cargo.toml` (compile only).
