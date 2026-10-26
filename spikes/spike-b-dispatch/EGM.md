# spike-b-dispatch

## Overview

Standalone comparison of a small portable match-dispatch VM with a function-pointer handler table.

## Interface

- Rust library and native binary: [src](./src/EGM.md).
- V8 runner: [run.mjs](./run.mjs).

## Invariants

- Both dispatch variants run the same hand-assembled program and track dispatched-instruction counts.
- The experiment has fixed internal bytecode and uses unchecked indexing based on that program shape.

## Side Effects

- Native and Wasm runners time execution; do not run locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-b-dispatch/Cargo.toml`; `node --check spikes/spike-b-dispatch/run.mjs`.
