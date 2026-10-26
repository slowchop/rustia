# alt-luau-wasm

## Overview

Small standalone mlua Luau smoke executable for evaluating a fixed script.

## Interface

- Implementation package: [src](./src/EGM.md), with entry point [src/main.rs](./src/main.rs).

## Invariants

- The script is fixed in source and returns a numeric sum.

## Side Effects

- Execution evaluates Luau and prints the sum.

## Verification

Run from repository root: `cargo check --manifest-path spikes/alt-luau-wasm/Cargo.toml`
