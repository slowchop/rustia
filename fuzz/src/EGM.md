# src

## Overview

Shared bounded fuzz harness library for frontend, VM, stateful embedding and differential cases.

## Interface

- [lib.rs](./lib.rs) defines lexer/parser/compiler/VM fuzz entry points and the opcode decoder.
- [stateful.rs](./stateful.rs) models 25 bounded embedding operations, including retained export/function replacement, rollback, root release, collection and exact reset-epoch checks.
- [differential.rs](./differential.rs) generates source and runs the oracle comparison.
- Oracle process entry: [bin](./bin/EGM.md).

## Invariants

- Lexer 64KiB, parser 32KiB, compiler 16KiB; VM 256 decoded instructions, 16 registers, 4096 fuel, recursion 32, 512KiB memory; stateful 128 bytes, 4096 fuel, 2MiB memory.
- VM target selects among all four profiles.
- Stateful harness checks a host-side model after each operation.
- Retained versions and captured values come from a fixed host array. Current/previous exports, one detached function and one stale export/function pair bound the persistent handle slots. A stored weak witness verifies actual quota-driven collection.

- Differential generation includes 57 bounded shapes. CI smoke shares `SOURCE_SHAPE_COUNT` with the generator and visits all eight type variants, covering valid/invalid assertions, alias defaults, pack defaults, generic function signatures, return annotations, table types and result adjustment. CI also crosses eight type leaves, eight wrappers and four source contexts (256 combinations). This is bounded compositional coverage, not full grammar parity.

## Side Effects

- Differential harness launches child processes with bounded input/output and a supervisor time limit.

## Verification

Run from repository root: `cargo check --locked --manifest-path fuzz/Cargo.toml --lib`.
