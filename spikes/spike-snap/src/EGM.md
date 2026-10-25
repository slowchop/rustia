# src

## Overview

Snapshot benchmark entry point.

## Interface

- [main.rs](./main.rs) defines sample state, step workload, snapshot variants, and timing output.

## Invariants

- The same representative state shape and update step drive the compared snapshot strategies.

## Side Effects

- Uses wall-clock timing and mutates local benchmark state.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-snap/Cargo.toml`.
