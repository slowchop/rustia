# spike-a-luau

## Overview

Standalone reference-Luau version of a grabby-shaped AI-kernel experiment.

## Interface

- Implementation and harness package: [src](./src/EGM.md), with entry point [src/main.rs](./src/main.rs).

## Invariants

- The realistic Luau path intentionally uses f64 as a performance proxy; it is not the deterministic fixed-point path.

## Side Effects

- Main runs native and Luau timing paths and prints measurements.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-a-luau/Cargo.toml` (compile only).
