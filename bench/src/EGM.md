# Native Benchmark Harness

## Overview

This directory contains the native Rust benchmark executable. The Cargo target and adapter feature set are defined in [`../Cargo.toml`](../Cargo.toml). Measurement belongs on the qualified `ci-bench` host, not on a developer machine ([benchmark policy](../../AGENTS.md#benchmark-policy)).

## Interface

- [`main.rs`](main.rs) is the internal `script-bench` CLI entrypoint, not Rustia's public embedding API. Engine adapters are feature-gated; the default feature is `rustia-engine` ([Cargo manifest](../Cargo.toml#L16)).
- Memory-child events use v2 schema metadata. `--memory-null-control` requires memory-child mode: it prepares and warms the same source but skips guest epoch execution while retaining every handshake. The supervisor supplies the common natural-idle interval before the quiescent event. Unit coverage proves the no-work path does not call the epoch executor.
- The VM string-memory binary is a separate target documented in [`bin/EGM.md`](bin/EGM.md).
- The normal Rustia adapter is behind `rustia-engine`. Its supplemental `string_opcodes` preparation compiles a top-level program with `BENCH_N`; a correctness test requires all nine specialized string opcodes. Other workloads retain their returned-function adapters.

## Invariants

- `ci-bench` alone is authorized to produce timing evidence; local builds are correctness checks only ([repository policy](../../AGENTS.md#benchmark-policy)).
- The suite compares per-workload results and validates checksums; the design calls for a fresh prepared engine/workload state before calibration and each recorded repetition ([bench design](../../design/80-bench-suite.md)).
- Builds and correctness checks do not provide performance evidence; only `ci-bench` does ([repository policy](../../AGENTS.md#benchmark-policy)).

## Side Effects

- Running the binary compiles and executes guest workloads and may perform timed measurement. Keep execution on the authorized benchmark workflow.

## Verification

Run from the repository root: `cargo check --locked --manifest-path bench/Cargo.toml --all-targets` checks the native harness build without running workloads.
