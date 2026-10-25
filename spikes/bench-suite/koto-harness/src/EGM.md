# src

## Overview

Koto baseline executable and optional API probe.

## Interface

- [main.rs](./main.rs) selects probe or benchmark behavior from command-line arguments.

## Invariants

- Workload results are kept observable with returned checksums.

## Side Effects

- Benchmark mode calibrates and times Koto workloads; do not execute locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/bench-suite/koto-harness/Cargo.toml`.
