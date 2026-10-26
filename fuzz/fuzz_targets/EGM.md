# fuzz_targets

## Overview

Thin libFuzzer entry points for each fuzz domain.

## Interface

- [compiler.rs](./compiler.rs), [differential.rs](./differential.rs), [lexer.rs](./lexer.rs), [parser.rs](./parser.rs), [stateful.rs](./stateful.rs), and [vm.rs](./vm.rs) delegate input to `rustia-fuzz` shared functions.

## Invariants

- Adapters map shared harness results to `Corpus::Keep` or `Corpus::Reject`; generated input logic is in the sibling library.

## Side Effects

- Running a target under cargo-fuzz may update corpus/artifact directories.

## Verification

Run from repository root: `cargo check --locked --manifest-path fuzz/Cargo.toml --lib` (wrapper binaries are selected by cargo-fuzz).
