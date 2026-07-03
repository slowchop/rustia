# Release Readiness Checklist

This checklist tracks what must be true before any public rustia crate flips
from `publish = false` to publishable.

## Current Policy

- Keep all workspace crates `publish = false` until the P1 engine, conformance,
  benchmark, and API gates below are complete.
- Semver and API stability rules are defined in
  [`91-semver-policy.md`](91-semver-policy.md).
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
- Package file lists are audited by `tools/check-package-files.js`; the
  committed review artifact is [`package-files.json`](package-files.json).
- Publishability blockers are audited by `tools/check-publishability.js`; the
  committed review artifact is [`publishability.json`](publishability.json).
- Internal crate dependencies have explicit versions before any crate is
  published.
- Leaf packages currently verified with `cargo package --no-verify`:
  `rustia-lexer` and `rustia-vm`.
- Full-workspace packaging is still blocked until internal crates are published
  in topological order. The current generated order is `rustia-lexer`,
  `rustia-parser`, `rustia-vm`, `rustia-compiler`, `rustia`, `rustia-cli`,
  then `rustia-conformance`; `rustia-lexer` and `rustia-vm` are the first
  candidates after the policy flip because they have no internal dependencies.
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
  `bench/current-results.json`, `bench/current-results.html`, and
  `bench/CLAIMS.md` are generated from the same committed benchmark history.
- Native baseline history covers the completed adapter matrix on a clean commit.
- Public benchmark claims are stated per workload in `bench/CLAIMS.md`, not as
  a single blanket multiplier. The current generated claim gate approves no
  global speed headline.
- Public benchmark workflows use only the generic `[self-hosted, bench]` runner
  label after owner setup is confirmed.

## Release Process Gate

- `just release-preflight` passes before any publishability flip. It checks
  release notes, package file lists, generated progress HTML, benchmark result
  freshness, and the workspace test suite.
- `.github/workflows/ci.yml` mirrors the release-preflight gate on hosted Linux
  CI without running fresh benchmark measurements.
- `.github/workflows/benchmarks.yml` provides manual current-matrix measurement
  on the generic `[self-hosted, bench]` runner label and uploads generated
  benchmark artifacts.
- `cargo test --workspace` passes on the release toolchain.
- `just check-package-files` passes and `design/package-files.json` is current.
- `just check-publishability` passes and `design/publishability.json` is
  current.
- `just check-release-notes` passes and `CHANGELOG.md` names the current
  workspace version plus public-facing unreleased changes.
- `just check-progress` and `just check-benchmark-results` pass without
  rewriting committed generated artifacts.
- `rustia-cli` supports file, stdin, and `--eval` execution, plus profile,
  fuel, and source-name options for diagnostics.
- `cargo package --no-verify` reaches packaging for each publishable crate; any
  unpublished path dependency blocker is documented.
- README, `design/CURRENT.md`, `design/progress.json`, and generated progress
  HTML agree on conformance counts, benchmark state, and release status.
- The crate graph has a documented semver policy before the first non-`0.0.x`
  release.
