# src

## Overview

One proof-of-concept binary for gc-arena constraints and serde.

## Interface

- [main.rs](./main.rs) defines the sample graph, exclusive Send wrapper, JSON conversion, and fuel-paced collection demo.

## Invariants

- String leaves are held through `Static`; collection is invoked at explicit step boundaries.

## Side Effects

- The executable spawns a thread and runs serde and collection scenarios.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-ga-gcarena/Cargo.toml`.
