# bin

## Overview

Oracle worker executable source used by the differential harness.

## Interface

- [error_oracle_worker.rs](./error_oracle_worker.rs) accepts `luau` or `rustia` mode and emits a compact result record.

## Invariants

- Accepts at most 16 KiB of source, reading one extra byte to detect oversize input; each worker handles one source per process.
- Oracle mode is gated by the Cargo `oracle` feature.

## Side Effects

- Reads stdin, executes source in one engine, and prints classification/value data.

## Verification

Run from repository root: `cargo check --locked --manifest-path fuzz/Cargo.toml --features oracle --bin error-oracle-worker`.
