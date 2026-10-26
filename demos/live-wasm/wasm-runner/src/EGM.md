# src

## Overview

Raw-Wasm ABI bridge for interactive execution and the demo benchmark protocol.

## Interface

- [lib.rs](./lib.rs) exports allocator/deallocator, `rustia_run`, result pointer/length, ABI version, benchmark prepare and benchmark run functions.

## Invariants

- Pointer and capacity preconditions are documented on unsafe exported functions.
- Result bytes live in thread-local storage and remain valid only according to that buffer’s lifetime.

## Side Effects

- Calls evaluate guest code, replace prepared state, invoke guest function, or update result bytes.

## Verification

Run from repository root: `cargo check --locked --manifest-path demos/live-wasm/wasm-runner/Cargo.toml --target wasm32-unknown-unknown`.
