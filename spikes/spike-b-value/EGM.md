# spike-b-value

## Overview

Standalone value-layout and heap representation experiment; compares NaN-boxed, tagged, and larger baseline values.

## Interface

- Core experiment package: [src](./src/EGM.md), with implementation [src/lib.rs](./src/lib.rs) and native reporter [src/main.rs](./src/main.rs).
- V8 runner: [run.mjs](./run.mjs).

## Invariants

- The same handle-based heap is shared across value layouts; insertion order is stored separately from hash lookup.
- NaN canonicalization protects the NaN-box tag space; signed zero is preserved for values and normalized only for numeric table keys.
- String interning deduplicates by hash without comparing bytes; source labels this as safe only for the spike corpus.

## Side Effects

- Native and Wasm harnesses perform large timing runs; do not execute locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-b-value/Cargo.toml`; `node --check spikes/spike-b-value/run.mjs`.
