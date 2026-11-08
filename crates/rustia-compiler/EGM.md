# rustia-compiler Package

## Overview

The compiler converts the parser's Luau AST into `rustia-vm` bytecode. It is a lower-level engine package; application users normally compile through the `rustia` facade.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents compile entry points and lowering modules.
- Depends on the lexer, parser, and VM crates.

## Invariants

- The compiler discovers captured locals and syntactic global writes before final lowering.
- Default-disabled O2 passes do not alter the default bytecode schedule.
- Logical receipt metadata lets optimized code retain the baseline logical instruction/fuel and deterministic-collection schedule.
- Luau type syntax is parsed as runtime-erased metadata; this compiler does not type-check programs.

## Side Effects

Compilation allocates AST/lowering/bytecode data and returns structured compile errors; it does not execute guest code or call host services.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
The feature-gated `tests/gc_accounting_paths.rs` test uses the actual closure and
generic-iteration workloads to report setup, warmup and following-call root-accounting/executor
counters at fixed work sizes. It asserts exact checksums and emits stable rows,
but performs no timing; run it only through its explicit release-mode
`executor-diagnostics` feature command in private self-hosted CI.
