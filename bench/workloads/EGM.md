# Benchmark Workloads

## Overview

This directory groups guest programs by source language. The direct workload sets are documented in [`javascript/EGM.md`](javascript/EGM.md), [`lua54/EGM.md`](lua54/EGM.md), and [`luau/EGM.md`](luau/EGM.md). Their operation shapes are designed for comparison by the harness; they are not standalone performance evidence.

## Interface

- Each workload file supplies an internal callable workload entry to its adapter, not a public library API. The entry receives a repetition count and returns a checksum or aggregate used by its caller; language-specific details are in the child maps.

## Invariants

- Preserve each source's operation count, state lifetime, and checksum when changing a workload; the checksum is the correctness guard for benchmark comparisons.
- Never execute these sources locally for timing. The repository's timing authority is `ci-bench` ([AGENTS.md](../../AGENTS.md#benchmark-policy)).

## Side Effects

- Workloads allocate and mutate guest-language tables, arrays, strings, closures, and application state. Retention controls intentionally keep state reachable; see the language maps.

## Verification

Run from the repository root: `just check-benchmark-results` validates committed results and runs offline harness self-tests; it does not execute this workload tree for performance measurements.
