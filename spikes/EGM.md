# spikes

## Overview

Container for independent historical experiments used to investigate Rustia architecture choices. These projects do not define production runtime behavior.

## Interface

- Child experiments: [alt-luau-wasm](./alt-luau-wasm/EGM.md), [bench-suite](./bench-suite/EGM.md), [spike-a-luau](./spike-a-luau/EGM.md), [spike-b-dispatch](./spike-b-dispatch/EGM.md), [spike-b-value](./spike-b-value/EGM.md), [spike-c-libm](./spike-c-libm/EGM.md), [spike-d-gc](./spike-d-gc/EGM.md), [spike-ga-gcarena](./spike-ga-gcarena/EGM.md), [spike-h-slice](./spike-h-slice/EGM.md), [spike-sk-rustia](./spike-sk-rustia/EGM.md), [spike-snap](./spike-snap/EGM.md), and [spike-wasmjit](./spike-wasmjit/EGM.md).

## Invariants

- Architecture decisions and CURRENT distinguish historical experiments from production implementation and public performance claims.

## Side Effects

- Individual executables may run CPU-intensive experiments, print timing data, or write generated Wasm artifacts.

## Verification

Run from repository root: No direct package command. Use each child package’s static `cargo check` command; do not run spike executables.
