# src

## Overview

Reference-Luau calibration and timing harness.

## Interface

- [main.rs](./main.rs) loads the 11 sibling Luau workloads through mlua and reports per-workload figures.

## Invariants

- One host call runs a calibrated batch; checksum results keep work observable; repetitions are summarized per workload.

## Side Effects

- Uses wall-clock timing and prints performance results.

## Verification

Run from repository root: `cargo check --manifest-path spikes/bench-suite/Cargo.toml` (compile only; do not run the binary).
