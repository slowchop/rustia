# src

## Overview

Experimental hand-compiled register VM and arena-backed value graph.

## Interface

- [lib.rs](./lib.rs) defines `Vm`, `Chunk`, host function ABI, owned values, execution, snapshot and deserialization.
- [main.rs](./main.rs) performs semantic assertions and timing reports.

## Invariants

- No source parser/compiler is present; callers construct `Chunk` bytecode directly.
- Arena-bound `Value` is copied only within the experiment; owned values form the host boundary.
- Tables retain insertion order.

## Side Effects

- VM operations mutate arena state and invoke host functions; harness performs timing and explicit collections.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-h-slice/Cargo.toml` (compile only).
