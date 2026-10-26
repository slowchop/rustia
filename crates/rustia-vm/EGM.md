# rustia-vm Package

## Overview

`rustia-vm` is the lower-level execution engine and runtime representation crate. It exposes bytecode, errors, profiles, extension hooks, numeric helpers, and VM operations. The `rustia` facade is the application-facing boundary.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents the crate source boundary and execution internals.
- `Vm` executes `Program` bytecode with explicit settings, services, fuel, and guest transactions.
- Public engine modules include bytecode, errors, extensions, instruction facts, liveness, numeric operations, profiles, and VM.

## Invariants

- One portable engine serves targets; no target-specific semantic engine is implied.
- Deterministic numeric operations canonicalize NaN values and normalize numeric table keys.
- Fuel and recursion exhaustion are structured runtime errors; memory quota is logical VM-owned memory, not process RSS.
- `DeterministicSim` and `HardenedLeaf` are distinct policy dimensions; hardened leaves impose a restricted guest surface.
- Low-level engine `Value`/`Vm` are not the facade's rooted `Value`/`Rustia` API contract.

## Side Effects

VM execution mutates the GC-managed guest heap and execution counters, may collect, and may call configured host functions, module resolvers, clocks, entropy providers, or stdout sinks.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm`.
