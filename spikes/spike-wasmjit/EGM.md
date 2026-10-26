# spike-wasmjit

## Overview

Historical runtime-Wasm-emission experiment for one numeric kernel, comparing interpreter, static AOT, generated code and shared-memory generated code.

## Interface

- Emitter and drivers: [src](./src/EGM.md).
- Native artifact emitter: [src/main.rs](./src/main.rs).
- V8 runner: [harness.mjs](./harness.mjs).

## Invariants

- Every execution route is intended to return the same checksum.
- Shared-memory variant imports the host linear memory.
- The native helper writes generated Wasm artifacts to its current working directory.

## Side Effects

- Native helper writes four `.wasm` files and performs timing; JS compiles, instantiates and times Wasm. Do not run locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-wasmjit/Cargo.toml`; `node --check spikes/spike-wasmjit/harness.mjs`.
