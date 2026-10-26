# src

## Overview

Generic value-layout, table/heap and kernel experiment.

## Interface

- [lib.rs](./lib.rs) defines `VmValue`, `Heap<V>`, `Table<V>`, value layouts and benchmark exports.
- [main.rs](./main.rs) reports sizes, soundness scan, golden and timing results.

## Invariants

- The intended comparison varies value encoding while keeping the handle heap model common.
- The experiment is not a production GC implementation.

## Side Effects

- Benchmark routines mutate test heaps/world state and execute large iteration counts.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-b-value/Cargo.toml` (compile only).
