# VM String Memory Probe

## Overview

This directory contains the separate `vm-string-memory` Rust binary used for evidence-only VM string-memory experiments. Cargo enables it through the `rustia-engine` feature ([manifest](../../Cargo.toml#L70)).

## Interface

- [`vm-string-memory.rs`](vm-string-memory.rs) is an internal experiment CLI, not a public embedding API. It accepts scenario, VM-count, round/sample identity, build role, commit, and protocol digest inputs. Scenarios are empty, short/long strings, and loaded/unloaded variants ([source](vm-string-memory.rs#L29)).

## Invariants

- The implementation caps VM count at 2,048 and represents the five named scenarios explicitly ([source](vm-string-memory.rs#L23)).
- Its memory snapshot check can reject nonzero swap ([source](vm-string-memory.rs#L258)).
- This experiment is evidence-only and must not be used to alter throughput history ([bench design](../../../design/80-bench-suite.md#automation-structure)).
- `ci-bench` alone is the timing authority; this binary must not be run locally for performance evidence ([repository policy](../../../AGENTS.md#benchmark-policy)).

## Side Effects

- The binary prints JSON sample records after Linux collection. It reads `/proc/self/status` and `/proc/self/smaps_rollup` for process memory snapshots and times VM/program construction and execution ([entrypoint](vm-string-memory.rs#L295), [sample collection](vm-string-memory.rs#L421), [memory snapshot](vm-string-memory.rs#L658)). `--help` writes usage to stdout; a failed run writes an error to stderr and exits with status 1 ([source](vm-string-memory.rs#L284)).

## Verification

Run from the repository root: `cargo check --locked --manifest-path bench/Cargo.toml --bin vm-string-memory` checks the binary without running an experiment.
