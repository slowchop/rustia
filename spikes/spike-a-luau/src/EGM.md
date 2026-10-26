# src

## Overview

Native kernel model and reference-Luau comparison harness.

## Interface

- [main.rs](./main.rs) defines Q16.16 helpers, `World`, Luau scripts, and benchmark functions.

## Invariants

- World uses fixed counts for actions, considerations, and neighbors; deterministic per-agent/tick random streams are derived from FNV and ChaCha8.
- The Luau f64 path is explicitly labeled a performance proxy.

## Side Effects

- Benchmark helpers execute timed native and mlua runs.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-a-luau/Cargo.toml` (does not run the experiment).
