# live-wasm

## Overview

Standalone bare-Wasm browser demo for evaluating Luau source with the standard preset and no stdout.

## Interface

- [index.html](./index.html) is the browser UI.
- Runner package: [wasm-runner](./wasm-runner/EGM.md).
- Build and serving notes: [README.md](./README.md).
- The sibling [JavaScript-services recipe](../web-live/README.md) uses wasm-bindgen and the live preset.

## Invariants

- Page instantiates raw Wasm with an empty import object; runner disables Rustia’s default `platform-adapters` feature.

## Side Effects

- Browser actions pass source to Wasm and display the returned output; serving uses a local static HTTP server.

## Verification

Run from repository root: Static runner check: `cargo check --locked --manifest-path demos/live-wasm/wasm-runner/Cargo.toml --target wasm32-unknown-unknown`.
