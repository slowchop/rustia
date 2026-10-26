# src

## Overview

Custom register bytecode VM and kernel builders for the historical walking skeleton.

## Interface

- [lib.rs](./lib.rs) exposes value helpers, `Program`, `Vm`, world model, builders and Wasm benchmark exports.
- [main.rs](./main.rs) contains the native timing/golden harness.

## Invariants

- Register count and bytecode layouts are fixed within this experiment.
- Fuel is charged per dispatched instruction; Q16 behavior is checked against a fixed checksum.

## Side Effects

- The native main and exported benchmark functions execute the kernel many times.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-sk-rustia/Cargo.toml` (compile only).
