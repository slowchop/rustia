# rustia Package

## Overview

`rustia` is the application-facing facade over the lexer, parser, compiler, and VM. It owns the builder and application settings/errors, typed conversions, host integration, diagnostics, and rooted handles. Applications can use the experimental 0.x API from Git. The crate is unpublished, and supported-release acceptance remains open.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents the facade implementation and exports.
- [`examples/EGM.md`](./examples/EGM.md) documents runnable usage examples.
- [compare.md](./compare.md) supplies the project comparison for the README and browser page.
- [performance.svg](./performance.svg) shows a dated ci-bench cohort for the README and browser comparison. Run `node tools/check-readme-comparisons.js` to verify the chart against retained history.
- The package is built with `cargo check -p rustia`; `web-live` is enabled by default for browser WebAssembly services.

## Invariants

- `OwnedValue` is the acyclic, Rust-owned copy boundary. Live `Value`, `Table`, `Function`, and `Exports` handles preserve identity and belong to one VM and decision epoch. Host-created live vectors remain immutable and use the same quota/root/epoch contract without entering `OwnedValue`.
- `reset_decision()` requires an idle VM, advances the epoch, preserves configuration and persistent registrations, and invalidates retained guest handles.
- The default facade is thread-affine. The opt-in `transferable` feature requires transferable service captures (`Send`, with `Fn + Send + Sync` for entropy) and `Send + Sync` object payloads and makes the interpreter/live handles movable between workers; it does not permit concurrent entry. A unified Cargo graph that enables `rustia-vm/send` must forward `rustia/transferable`.
- Typed invocations borrow their `HostContext` for one call, share a fuel budget, and publish bounded ordered records only after successful completion.
- Typed Rust objects expose synchronous, reset-safe length and generalized-iterator hooks with ordinary fuel, quota, diagnostics and transferable bounds.
- `install` rolls back guest state on failure. Ordinary evaluation mutations and external host effects are not undone.
- The facade boundary must not expose internal bytecode, raw heap identities, or collector lifetimes.

## Side Effects

Running a chunk mutates guest state. Installation can call module resolvers and host functions. Reset discards decision-local guest state. Host services, Rust payload mutation, and I/O remain external effects.

## Verification

From the repository root, run `cargo test --locked -p rustia`. To compile examples without running them, use `cargo check --locked -p rustia --examples`.
