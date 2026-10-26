# Benchmark Harness

## Overview

This directory holds the benchmark crate, workload programs, memory-analysis tools, and generated reports. The Rust crate defaults to the `rustia-engine` feature and keeps competing engines behind separate Cargo features. Local source checks and artifact validation are correctness work; `ci-bench` alone is the timing authority. See the [harness design](../design/80-bench-suite.md) and [canonical project state](../design/CURRENT.md).

## Interface

- `Cargo.toml` defines the `script-bench` and `vm-string-memory` binaries and adapter features.
- [`run-memory-cohort.js`](run-memory-cohort.js) accepts a protocol config and two binaries, then coordinates evidence generation. [`cgroup-delegation.js`](cgroup-delegation.js) locates the usable delegated cgroup-v2 ancestor; [`publish-memory-analysis.js`](publish-memory-analysis.js) validates and writes memory cohort artifacts.
- The native harness and VM memory probe live in [`src/EGM.md`](src/EGM.md). Workload sources are grouped in [`workloads/EGM.md`](workloads/EGM.md). Wasm drivers and adapter code are in [`wasm/EGM.md`](wasm/EGM.md). Grabby build reporting is in [`grabby-builds/EGM.md`](grabby-builds/EGM.md).
- Supporting direct sources include [`run-memory-supervisor.js`](run-memory-supervisor.js), [`analyze-memory-samples.js`](analyze-memory-samples.js), [`analyze-fixed-samples.js`](analyze-fixed-samples.js), [`render-current-results.js`](render-current-results.js), and [`render-claims.js`](render-claims.js). These are internal harness and report tools, not embedding APIs.

## Invariants

- Benchmark comparisons preserve workload checksums and use fresh prepared state for calibration and recorded repetitions, as specified by the [bench design](../design/80-bench-suite.md).
- A memory cohort is tied to one host label. `run-memory-cohort.js` validates the protocol's exact shape and label, and the publisher rejects rows from multiple hosts ([coordinator](run-memory-cohort.js), [publisher](publish-memory-analysis.js)).
- Memory measurement fails closed without a writable cgroup-v2 ancestor whose root is empty and has the memory controller ([probe](cgroup-delegation.js)).
- Never run performance measurements locally. Only `ci-bench` results count as benchmark evidence ([repository policy](../AGENTS.md#benchmark-policy), [current state](../design/CURRENT.md)).

## Side Effects

- The cohort coordinator starts child processes and writes evidence to its configured output directory. The supervisor reads Linux process and cgroup memory counters while a measured child runs.
- The publisher writes history and current-result JSON atomically to configured paths.
- Harness binaries execute guest workloads and can measure elapsed time. Do not use those execution paths for local performance evidence.

## Verification

Run from the repository root: `cargo check --locked --manifest-path bench/Cargo.toml --all-targets` checks the harness build. `just check-benchmark-results` runs offline tool self-tests and validates committed artifacts; it does not establish performance results.
