# src

## Overview

Library implementation of the f64/libm identity probe plus native printer.

## Interface

- [lib.rs](./lib.rs) exposes `compute() -> Report`, `FN_NAMES`, and Wasm exports `wasm_n`/`wasm_hash`.
- [main.rs](./main.rs) prints native report data.

## Invariants

- Input set, function ordering and FNV hashing are deterministic.
- The probe’s canonical hash folds NaN and both signed zeros to canonical patterns.

## Side Effects

- The binary emits output; library `compute` calculates hashes without file or network access.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-c-libm/Cargo.toml`.
