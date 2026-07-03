# Release Readiness Checklist

This checklist tracks what must be true before any public rustia crate flips
from `publish = false` to publishable.

## Current Policy

- Keep all workspace crates `publish = false` until the P1 engine, conformance,
  benchmark, and API gates below are complete.
- Keep `script-bench-harness` unpublished while it depends on unpublished
  workspace path crates for the optional rustia adapter.
- Use dual `MIT OR Apache-2.0` licensing for every public crate.
- Keep public rustia APIs generic. Do not publish grabby-specific host types,
  effect catalogs, or integration code from this repository.
- Treat Luau 0.709 conformance as the compatibility contract until the target is
  deliberately re-pinned.

## Crate Metadata Gate

- Every public workspace crate has description, repository, homepage, readme,
  license, keywords, categories, and rust-version metadata.
- Package file lists must not include private design scratch, `private/`, local
  caches, or generated target output.
- Internal crate dependencies have explicit versions before any crate is
  published.
- Leaf packages currently verified with `cargo package --no-verify`:
  `rustia-lexer` and `rustia-vm`.
- Full-workspace packaging is still blocked until internal crates are published
  in topological order; today `rustia-parser` correctly resolves
  `rustia-lexer ^0.0.1` from crates.io and fails because that crate is not yet
  published.
- The benchmark crate must either publish the `rustia-*` crates first, move the
  rustia adapter into a git/path-only integration crate, or ship that adapter as
  an example outside the core package.

## API Gate

- Public error types, conversion traits, retained handles, extension APIs, and
  hot-reload semantics have stable names and documented behavior.
- Capability checks fail closed in deterministic profiles.
- Host functions have documented fuel costs and diagnostic behavior.
- Public examples cover globals, typed functions, serde, extensions, retained
  functions, and hot reload.
- Current examples:
  - `crates/rustia/examples/embedding.rs` covers globals, typed host functions,
    deterministic extensions, retained script functions, and serde.
  - `crates/rustia/examples/hot_reload.rs` covers compile/install hot reload
    and retained handles across reload failures.

## Runtime Gate

- The Luau 0.709 conformance corpus remains pending-free.
- Profile behavior is covered for LuauCompat, ReproducibleStandard, and
  DeterministicSim.
- Fuel accounting has versioned costs and superinstructions remain
  fuel-invariant.
- GC pacing is tuned for Standard profiles, and DeterministicSim collection
  intervals are treated as deterministic ABI.
- JSC and SpiderMonkey wasm measurements are recorded before the value-layout
  and dispatch lock.

## Benchmark And CI Gate

- `bench/history/runs.jsonl`, `bench/RESULTS.md`,
  `bench/current-results.json`, and `bench/current-results.html` are generated
  from the same committed benchmark history.
- Native baseline history covers the completed adapter matrix on a clean commit.
- Public benchmark claims are stated per workload, not as a single blanket
  multiplier.
- Public benchmark workflows use only the generic `[self-hosted, bench]` runner
  label after owner setup is confirmed.

## Release Process Gate

- `cargo test --workspace` passes on the release toolchain.
- `cargo package --list` has been reviewed for every crate intended for
  publication.
- `rustia-cli` supports file, stdin, and `--eval` execution, plus profile,
  fuel, and source-name options for diagnostics.
- `cargo package --no-verify` reaches packaging for each publishable crate; any
  unpublished path dependency blocker is documented.
- README, `design/CURRENT.md`, `design/progress.json`, and generated progress
  HTML agree on conformance counts, benchmark state, and release status.
- The crate graph has a documented semver policy before the first non-`0.0.x`
  release.
