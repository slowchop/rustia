# src

## Overview

Two standalone GC experiment binaries; no production API.

## Interface

- [main.rs](./main.rs) compares barriers, GC cost, retained roots, serialization and snapshots.
- [miri_send.rs](./miri_send.rs) tests a thread-transfer scenario.

## Invariants

- Mutation and collection are separate arena operations.
- Unsafe Send justification requires moving the arena and its derived root handles together without concurrent aliases.

## Side Effects

- The main binary times operations; the second starts threads and collects garbage.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-d-gc/Cargo.toml --all-targets` (compile only).
