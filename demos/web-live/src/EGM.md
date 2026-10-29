# Browser Example Source

## Overview

Application glue between Rustia's embedding API and wasm-bindgen JavaScript exports.

## Interface

- [lib.rs](./lib.rs) exports `run_example() -> Result<i32, JsValue>` to JavaScript.

## Invariants

- The application constructs one fresh bounded `live()` interpreter per call.
- Source diagnostics become JavaScript exceptions; no public Rustia API is added.
- Source code forbids unsafe Rust.

## Side Effects

Script `print` uses the browser console. The function returns a copied integer.

## Verification

Build and run the [parent recipe](../README.md) and its pinned browser correctness check.

Parent map: [browser example](../EGM.md).
