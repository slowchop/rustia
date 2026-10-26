# src

## Overview

Wasm module generation library and native helper.

## Interface

- [lib.rs](./lib.rs) generates module bytes and exports interpreter/AOT/codegen paths.
- [main.rs](./main.rs) checks checksums and writes generated modules.

## Invariants

- Generated kernel follows the same deterministic input/checksum scheme as the comparison paths.

## Side Effects

- `main.rs` writes `gen_run.wasm`, `gen_run_opt.wasm`, `gen_shared.wasm`, and `gen_shared_opt.wasm` in the working directory.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-wasmjit/Cargo.toml` (compile only).
