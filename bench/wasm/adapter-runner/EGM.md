# Wasm Adapter Crate

## Overview

This standalone Cargo crate builds the wasm `cdylib` used by the measurement drivers. Its Rust implementation is documented in [`src/EGM.md`](src/EGM.md).

## Interface

- [`Cargo.toml`](Cargo.toml) declares `script-bench-wasm-adapter`, a `cdylib`, with Rustia, Koto, Rhai, and Wasmi dependencies.

## Invariants

- `ci-bench` alone is authorized to produce timing evidence; this crate's build check is not a measurement ([repository policy](../../../AGENTS.md#benchmark-policy)).
- Release profile settings are part of the built wasm artifact; this crate is excluded from publication (`publish = false`) ([manifest](Cargo.toml)).
- Driver/adapter compatibility is versioned through the ABI exported by the library ([source map](src/EGM.md)).

## Side Effects

- Building creates Cargo target artifacts. The library's exported calls manage benchmark preparation and execution; do not call them for local timing.

## Verification

Run from the repository root: `cargo check --locked --manifest-path bench/wasm/adapter-runner/Cargo.toml` checks the crate without running a wasm workload.
