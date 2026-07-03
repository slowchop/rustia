# Standalone Split Notes

`script-bench-harness` is kept inside the rustia workspace while rustia is early,
but it is structured to become a standalone OSS benchmark project.

## Split Contract

- Keep `bench/history/runs.jsonl` as the append-only source of truth.
- Keep `bench/current-results.json`, `bench/RESULTS.md`, and
  `bench/current-results.html` generated from committed history.
- Keep `bench/site/` static and dependency-free so history can be served from a
  plain repository or static host.
- Keep adapter features independent. `mlua-lua54` must stay in a separate binary
  from `mlua-luau` because `mlua-sys` links both as `lua`.
- Keep dirty benchmark records out of committed history. `just benchmarks`
  captures a clean HEAD commit before the first history append and passes it to
  both runner binaries.

## Workspace Seams

The manifest is self-contained for package metadata and lints. The remaining
workspace coupling is intentional:

- The optional `rustia-engine` adapter depends on sibling path crates under
  `../crates/`.
- The root `Justfile` owns convenience recipes while the project remains in this
  workspace.
- `.github/workflows/benchmarks.yml` owns the current in-repo manual
  self-hosted measurement job while the project remains in this workspace.
- Root license files cover the current in-repo copy.

When the project splits, either publish `rustia-*` crates first, move the rustia
adapter behind an optional git/path integration crate, or keep it as an example
adapter outside the core benchmark package.

## Package Blocker

`cargo package -p script-bench-harness --no-verify` currently reaches the
packaging phase, but it cannot complete while the optional `rustia-*` adapter
dependencies are unpublished workspace path crates. That is expected until the
split chooses one of the rustia-adapter strategies above.
