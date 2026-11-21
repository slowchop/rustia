# fuzz

## Overview

Independent cargo-fuzz workspace with bounded frontend, VM, stateful API and differential targets.

## Interface

- [upstream_compiler.rs](tests/upstream_compiler.rs) compares scalar return values and compile-time rejections with pinned Luau. The cases live in [compiler0709](tests/compiler0709).
- [upstream_parser.rs](tests/upstream_parser.rs) compares the shared 0.709 parser examples with the pinned Luau compiler. The test enables the explicit type argument syntax used by Rustia. It never executes the scripts.
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
