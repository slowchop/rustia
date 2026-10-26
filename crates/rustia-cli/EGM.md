# rustia-cli Package

## Overview

This package builds the `rustia` executable. It provides single-file, stdin, `--eval`, and interactive REPL execution over the public facade.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents command parsing and execution.
- Binary target: `rustia` at `src/main.rs`.

## Invariants

- CLI profile and optional fuel settings flow through `RustiaBuilder`.
- Filesystem-backed `require` is omitted for the `DeterministicSim` profile.
- Source labels are retained for runtime diagnostics; shebang removal preserves line layout.

## Side Effects

The executable reads source from files or stdin, resolves module paths against the source directory/current directory, writes guest output and diagnostics, and returns process exit codes.

## Verification

From the repository root, run `cargo test --locked -p rustia-cli`.
