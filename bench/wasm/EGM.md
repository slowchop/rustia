# Wasm Benchmark Drivers

## Overview

This directory contains drivers for Rustia's wasm adapter and dispatch experiments, alongside committed result data and adapter notes. The comparison adapter crate is documented in [`adapter-runner/EGM.md`](adapter-runner/EGM.md).

## Interface

- [`measure-rustia-wasm.mjs`](measure-rustia-wasm.mjs), [`measure-rustia-wasm-sm.js`](measure-rustia-wasm-sm.js), and [`measure-dispatch-wasm.js`](measure-dispatch-wasm.js) are runtime-specific drivers. The SpiderMonkey driver accepts wasm path, commit/runtime/host metadata, adapter selection, target duration, repetitions, and max work; it validates exports and ABI version before preparation.
- Drivers use the adapter runner's exported ABI; see [`adapter-runner/src/EGM.md`](adapter-runner/src/EGM.md).

## Invariants

- The adapter identifiers and ABI version must match the wasm exports. The SpiderMonkey driver expects ABI version 1 and checks the required exports before running ([driver](measure-rustia-wasm-sm.js#L183)).
- These files implement measurement; local execution is prohibited by repository policy. `ci-bench` alone supplies performance evidence ([AGENTS.md](../../AGENTS.md#benchmark-policy), [CURRENT.md](../../design/CURRENT.md)).

## Side Effects

- Invoking a driver loads wasm, prepares an adapter, executes timed work, and emits measurement output. Do not invoke as a local check.

## Verification

Run from the repository root: `node --check bench/wasm/measure-rustia-wasm.mjs` and `node --check bench/wasm/measure-dispatch-wasm.js` perform syntax checks only. `cargo check --locked --manifest-path bench/wasm/adapter-runner/Cargo.toml` checks the adapter crate without a measurement run.
