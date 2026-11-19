# Browser JavaScript Services Example

## Overview

A standalone application showing browser embedding with Rustia's default `web-live`
feature, the explicit `live()` preset, and wasm-bindgen's generated JavaScript glue.

## Interface

- [Cargo.toml](./Cargo.toml) and Cargo.lock pin the standalone application dependencies.
- [src](./src/EGM.md) exports the application's `run_example` JavaScript function.
- [index.html](./index.html) initializes the generated module and displays the result.
- [hot-reload.html](./hot-reload.html) keeps one Wasm session while the user edits its movement rule. Capture mode advances the same session by explicit ticks.
- [race.gif](./race.gif) and [hot-reload.gif](./hot-reload.gif) are the two recordings embedded in the project README.
- [race.luau](./race.luau) defines three racers' progress; [race-native.rs](./race-native.rs) emits distances for the native half of the recording.
- [README.md](./README.md) contains the build, serve, and verification recipe.

## Invariants

- This application is outside the root workspace and remains unpublished.
- Default features link JavaScript services; `live()` explicitly selects live inputs/output.
- Generated files in `pkg/` and `target/` are build output, not source.
- Guest execution uses explicit fuel and accounted-memory limits. No timing is collected.

## Side Effects

Building writes Wasm and generated JavaScript. Serving opens a loopback HTTP endpoint.
Clicking the page constructs a fresh VM, prints to the browser console, and displays `42`.

## Verification

Follow [README.md](./README.md). The existing pinned WebKit CI job builds and executes
the page through [the correctness check](../../tools/browser/web-live-example.cjs).
The [headless recorder](../../tools/browser/record-race.cjs) checks both race traces and encodes the fixed-step replay.

Parent map: [demos](../EGM.md).
