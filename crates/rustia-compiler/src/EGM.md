# Compiler Source

## Overview

[lib.rs](./lib.rs) contains the compiler API and lowering passes. [builtins.rs](./builtins.rs) contains recognized global/library names that select specialized call evaluation order. [o2.rs](./o2.rs) contains default-disabled optimization rewrites and logical receipts.

## Interface

- `compile(&Chunk) -> Result<Program>` uses default options.
- `compile_with_options` accepts source metadata and PUC constructor-order settings.
- `compile_with_options_and_receipts` also returns `ProgramReceipts` for rewritten code.
- `CompileError` carries an optional lexer span; `BuiltinEnvironment` separates public and conformance compilation modes.
- `ExperimentalO2Options::reachability_exact()` selects the documented corpus-green rewrite track; `ALL` includes dead cleanup-clear elision, which the source marks as blocked on root-liveness work.

## Invariants

- Capture discovery is a whole-tree prepass so assignment sites account for captures and source-later global writes.
- O2 is all-off by default. Rewrites must preserve evaluation/error order and logical schedule.
- Do not treat the experimental option fields as a stable public compiler contract.

## Side Effects

Compilation is in-memory and does not execute the resulting program or perform host I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
