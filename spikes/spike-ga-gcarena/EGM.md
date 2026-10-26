# spike-ga-gcarena

## Overview

Standalone gc-arena proof of concept for Send constraints, serde bridging, fuel-paced collection and API ergonomics.

## Interface

- Implementation package: [src](./src/EGM.md), with entry point [src/main.rs](./src/main.rs).

## Invariants

- Arena-branded values stay within mutation closures; JSON-to-arena construction receives `Mutation`.
- The unsafe Send wrapper relies on exclusive arena ownership and does not expose metrics handles.

## Side Effects

- Main moves a wrapper to a thread, serializes/deserializes a sample graph, and runs collection steps.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-ga-gcarena/Cargo.toml --all-targets`. The `prove_not_send` feature is an expected compile-fail probe, not a passing check.
