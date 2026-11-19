# demos

## Overview

Standalone browser examples for bare Wasm and JavaScript host services.

## Interface

- [live-wasm](./live-wasm/EGM.md) contains the bare-Wasm standard-preset page and runner.
- [web-live](./web-live/EGM.md) contains the wasm-bindgen application, live services recipe, and seeded native/WebKit race demo.

## Invariants

- Both demo packages are outside root Cargo workspace membership and remain unpublished.
- The JavaScript-services recipe is correctness-only; local timing remains prohibited.

## Side Effects

- Serving the demo opens a local HTTP endpoint; building produces a Wasm artifact.

## Verification

Run from repository root: follow the child maps for their locked builds. The pinned WebKit CI job builds and executes the JavaScript-services recipe.
