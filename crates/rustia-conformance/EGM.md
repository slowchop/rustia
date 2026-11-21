# rustia-conformance Package

## Overview

This crate runs Rustia against the checked-in conformance manifest and fixtures, validates fixture/reference metadata, and summarizes oracle status. It also provides a command-line runner.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents library and CLI entry points.
- `run_repository` runs the repository corpus; `run_repository_case` selects one case; `list_repository_pending` lists pending oracle/implementation cases.
- The crate bundles the corpus in its package archive.

## Invariants

- Fixture identity, references, and status are validated before results are reported. Provenance tests validate every comma-separated upstream path, including the explicitly allowed pinned parser and generic-inference test sources.
- Fixture provenance recognizes the pinned CLI's flags-off invocation and the explicit-type-instantiation flag for the fixture that uses it.
- The report counts corpus state; it is not a general claim of full Luau compatibility.
- Experimental O2 conformance runs compile fixture programs with the selected options; source comments specify that preludes and `loadstring` remain on default lowering.

## Side Effects

The runner reads corpus manifests/reference and fixture files and executes guest programs. The binary prints result summaries and uses a dedicated large-stack thread.

## Verification

From the repository root, run `cargo test --locked -p rustia-conformance`.
