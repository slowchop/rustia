# spike-c-libm

## Overview

Cross-target f64/libm bit-pattern probe with native and Wasm reporting paths.

## Interface

- Library package: [src](./src/EGM.md), including [src/lib.rs](./src/lib.rs) and [src/main.rs](./src/main.rs); Wasm report: [run_wasm.mjs](./run_wasm.mjs).
- Captured prior outputs: [out_aarch64_mac.txt](./out_aarch64_mac.txt), [out_wasm32.txt](./out_wasm32.txt), [out_x86_64_linux.txt](./out_x86_64_linux.txt).

## Invariants

- The input sweep and hash order are fixed.
- Raw bits and canonicalized bits are reported separately; this experiment also canonicalizes negative zero, which is not the current production per-operation rule.

## Side Effects

- Executables print reports; the JS runner reads the compiled Wasm module.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-c-libm/Cargo.toml`; `node --check spikes/spike-c-libm/run_wasm.mjs`.
