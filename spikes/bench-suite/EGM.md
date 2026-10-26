# bench-suite

## Overview

Standalone reference Luau benchmark suite and Koto baseline harness. Its recorded results are historic comparison data, not current Rustia performance claims.

## Interface

- [README.md](./README.md) documents oracle versions, method, workloads, and run commands.
- Rustia source is absent by design; workloads live in [luau](./luau/EGM.md) and the harness in [src](./src/EGM.md).
- Koto comparison is in [koto-harness](./koto-harness/EGM.md).

## Invariants

- Each benchmark workload returns a checksum.
- Workload units differ; compare engines only within corresponding workload rows.

## Side Effects

- Running either harness performs local timings; repository policy prohibits local benchmark runs.

## Verification

Run from repository root: Static compile only: `cargo check --manifest-path spikes/bench-suite/Cargo.toml`.
