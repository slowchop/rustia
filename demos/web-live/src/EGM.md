# Browser Example Source

## Overview

Application glue between Rustia's embedding API and wasm-bindgen JavaScript exports.

## Interface

- [lib.rs](./lib.rs) exports `run_example() -> Result<i32, JsValue>` to JavaScript.
- Both the native example and the `race_frames` Wasm export call `race`, which runs the embedded Luau race in a new bounded deterministic VM.
- [live_racer.rs](./live_racer.rs) owns the editor's persistent VM, position and active movement function. A valid replacement export swaps the handle; a syntax error leaves the previous one installed.

## Invariants

- The services example constructs one fresh bounded `live()` interpreter per call.
- Source diagnostics become JavaScript exceptions; no public Rustia API is added.
- Source code forbids unsafe Rust.

## Side Effects

Script `print` uses the browser console. The function returns a copied integer.

## Verification

Build and run the [parent recipe](../README.md) and its pinned browser correctness check.

Parent map: [browser example](../EGM.md).
