# spike-sk-rustia

## Overview

Historical walking-skeleton VM and AI-kernel comparison with native and Wasm runners.

## Interface

- Core VM: [src](./src/EGM.md).
- Native reporter: [src/main.rs](./src/main.rs).
- Wasm runners: [run.mjs](./run.mjs) and [qcheck.mjs](./qcheck.mjs).

## Invariants

- Fixed-width custom bytecode uses a match dispatch loop and per-instruction fuel.
- Value is an 8-byte NaN-box; the f64 path is a performance proxy, while the Q16 path uses the fixed golden.
- This skeleton is an experiment, not the production runtime.

## Side Effects

- Native and JS scripts perform timing or golden execution against a built Wasm artifact.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-sk-rustia/Cargo.toml`; `node --check spikes/spike-sk-rustia/run.mjs`; `node --check spikes/spike-sk-rustia/qcheck.mjs`.
