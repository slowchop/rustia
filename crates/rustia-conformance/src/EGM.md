# Conformance Source

## Overview

[lib.rs](./lib.rs) reads the manifest/reference catalog, validates fixture metadata, runs all or selected fixtures, and returns reports. [main.rs](./main.rs) exposes full-run, case-selection, and pending-list commands.

## Interface

- `run_repository` returns fixture and oracle-status counts.
- `run_repository_with_experimental_o2` runs fixtures with explicit compiler rewrite options.
- `run_repository_case` runs a unique fixture selector; `list_repository_pending` returns pending cases.
- `bundled_repository` and `repository_from_crate_manifest` locate the included corpus.

## Invariants

- An empty manifest is rejected.
- A case selector must match exactly one ID or source path.
- Oracle status and cross-references are taken from the repository manifest/catalog, not inferred from a pass count.

## Side Effects

The library reads manifests, catalogs, and fixture source from disk and executes guest programs. The binary prints to stdout/stderr; its runner thread uses a 128 MiB stack.

## Verification

From the repository root, run `cargo test --locked -p rustia-conformance`.
