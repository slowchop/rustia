# wasm-runner

## Overview

Separate Rustia cdylib package that exposes the demo’s raw-Wasm interface.

## Interface

- Implementation package: [src](./src/EGM.md), with ABI implementation [src/lib.rs](./src/lib.rs).
- Browser caller: [../index.html](../index.html).

## Invariants

- The interactive engine uses the standard builder preset with stdout disabled; the package disables `web-live`.
- Unsafe ABI pointer contracts require valid caller-owned buffers.
- Engine, benchmark state and result buffer are thread-local.

## Side Effects

- Guest runs update engine/result state; benchmark preparation installs the requested function and repeated calls execute it.

## Verification

Run from repository root: `cargo check --locked --manifest-path demos/live-wasm/wasm-runner/Cargo.toml --target wasm32-unknown-unknown`.
