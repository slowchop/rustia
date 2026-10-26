# src

## Overview

Single entry point for the alternate mlua Luau experiment.

## Interface

- [main.rs](./main.rs) creates a Lua state, evaluates the loop, and prints its result.

## Invariants

- No external source or command-line configuration is read.

## Side Effects

- The binary evaluates a Luau chunk and writes one line to stdout.

## Verification

Run from repository root: `cargo check --manifest-path spikes/alt-luau-wasm/Cargo.toml`
