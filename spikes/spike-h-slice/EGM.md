# spike-h-slice

## Overview

Historical hand-compiled general-script vertical slice with gc-arena heap, tagged values, host calls and serde boundaries.

## Interface

- Core library and native harness: [src](./src/EGM.md).
- Wasm reporting harness: [run.mjs](./run.mjs).

## Invariants

- The program is hand-compiled bytecode, not a parser/compiler pipeline.
- The slice uses a tagged enum and `gc-arena`; it canonicalizes NaN per numeric result, preserves negative zero for values, and normalizes it for table keys.
- Host functions exchange owned, arena-free values.

## Side Effects

- Harnesses execute repeated VM runs, collection, serde and local timing; do not run them.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-h-slice/Cargo.toml`; `node --check spikes/spike-h-slice/run.mjs`.
