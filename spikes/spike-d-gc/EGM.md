# spike-d-gc

## Overview

Standalone gc-arena integration experiment comparing barriers, collection, retained roots, snapshots and slotmap behavior.

## Interface

- Implementation: [src](./src/EGM.md).

## Invariants

- Dynamic roots retain values across collection.
- The `miri_send` binary documents a sole-owner transfer invariant for its unsafe Send wrapper; that invariant is specific to the experiment.

## Side Effects

- Binaries run GC and thread experiments and emit measurements; do not execute locally.

## Verification

Run from repository root: `cargo check --manifest-path spikes/spike-d-gc/Cargo.toml --all-targets` (compile only).
