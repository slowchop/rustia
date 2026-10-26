# koto-harness

## Overview

Independent Koto baseline and capability-probe package.

## Interface

- Implementation package: [src](./src/EGM.md), with entry point [src/main.rs](./src/main.rs).
- Parent methodology and limitations: [../README.md](../README.md).

## Invariants

- The documented Koto subset is not equivalent to every Luau workload.

## Side Effects

- Normal mode times workloads; probe mode evaluates Koto capabilities. Neither mode is a static check.

## Verification

Run from repository root: `cargo check --manifest-path spikes/bench-suite/koto-harness/Cargo.toml`.
