# Standalone Split Notes

`script-bench-harness` is kept inside the rustia workspace while rustia is early,
but it is structured to become a standalone OSS benchmark project.

## Split Contract

- Keep `bench/history/runs.jsonl` as the append-only source of truth.
- Keep `bench/current-results.json`, `bench/RESULTS.md`, and
  `bench/current-results.html` generated from committed history.
- Keep `bench/wasm-gate.json` as the explicit pre-lock wasm measurement gate;
  recurring runs use Node/V8 and the accepted V8/JSC/SpiderMonkey matrix remains
  in `bench/wasm/history/` as portability evidence.
- Keep `bench/current-results.html` standalone and dependency-free so the
  generated report can be served from a plain repository or static host.
- Keep adapter features independent. `mlua-lua54` must stay in a separate binary
  from `mlua-luau` because `mlua-sys` links both as `lua`.
- Keep active timing records Linux/x86_64-only and sourced from a single
  qualified benchmark host. macOS and Windows remain supported build targets,
  but are outside the performance product until dedicated hosts exist.
- Keep accepted fixed-work raw sidecars and provenance manifests under
  `history/evidence/<full-commit>/`; they complement rather than extend the
  stable JSONL summary schema.
- Keep `analyze-fixed-samples.js` dependency-free and retain its exact seeded
  pooled/hierarchical analysis output beside accepted sidecars so the
  conservative regression verdict is reproducible after the split.
- Keep memory results in a separate versioned ledger under `bench/memory/`.
  Memory samples use one fresh process/cgroup per engine and workload, while
  the existing throughput history remains uninstrumented. The protocol and
  dashboard contract are defined in `design/57-memory-benchmarking.md`.

## Workspace Seams

The manifest is self-contained for package metadata and lints. The remaining
workspace coupling is intentional:

- The optional `rustia-engine` adapter depends on sibling path crates under
  `../crates/`.
- The root `Justfile` owns convenience recipes while the project remains in this
  workspace.
- The root `Justfile` and `tools/run-dedicated-benchmark.js` own the explicit
  Linux measurement lifecycle on the owned host. Hosted CI checks committed
  artifacts and never records fresh timings.
- Root license files cover the current in-repo copy.

When the project splits, either publish `rustia-*` crates first, move the rustia
adapter behind an optional git/path integration crate, or keep it as an example
adapter outside the core benchmark package.

## Package Blocker

`cargo package -p script-bench-harness --no-verify` currently reaches the
packaging phase, but it cannot complete while the optional `rustia-*` adapter
dependencies are unpublished workspace path crates. That is expected until the
split chooses one of the rustia-adapter strategies above.
