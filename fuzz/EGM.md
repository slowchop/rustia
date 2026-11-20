# fuzz

## Overview

Independent cargo-fuzz workspace with bounded frontend, VM, stateful API and differential targets.

## Interface

- Target adapters: [fuzz_targets](./fuzz_targets/EGM.md).
- Shared harness logic: [src](./src/EGM.md).
- [README.md](./README.md) documents input bounds, the hosted PR smoke and manually dispatched campaigns.

## Invariants

- Each target has explicit input/resource bounds; VM mutator includes production opcodes.
- Differential runs are correctness checks against a separately built Luau oracle; resource-policy equivalence is not inferred.

## Side Effects

- Fuzzing can write corpus and artifact files; oracle mode starts child processes.

## Verification

Run from repository root: `cargo check --locked --manifest-path fuzz/Cargo.toml --lib`; `cargo test --locked --manifest-path fuzz/Cargo.toml`.
