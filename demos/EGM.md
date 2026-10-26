# demos

## Overview

Grouping directory for the standalone Live Wasm browser demo.

## Interface

- [live-wasm](./live-wasm/EGM.md) contains the static page and Wasm runner.

## Invariants

- The demo package is outside root Cargo workspace membership.

## Side Effects

- Serving the demo opens a local HTTP endpoint; building produces a Wasm artifact.

## Verification

Run from repository root: No direct build target; use the child runner check.
