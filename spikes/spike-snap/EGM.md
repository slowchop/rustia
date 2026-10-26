# spike-snap

## Overview

Standalone snapshot/restore data-structure comparison for a grabby-shaped agent state.

## Interface

- Implementation and harness package: [src](./src/EGM.md), with entry point [src/main.rs](./src/main.rs).

## Invariants

- The workload models 4,000 agents and a 24-entry rollback ring; all state variants are mutated between snapshots.

## Side Effects

- The executable compares timed clone, persistent-collection and serialization paths.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-snap/Cargo.toml` (compile only).
