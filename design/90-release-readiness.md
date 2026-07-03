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
- Crate metadata is audited by `tools/check-crate-metadata.js`; the committed
  review artifact is [`crate-metadata.json`](crate-metadata.json).
- Package file lists must not include private design scratch, `private/`, local
  caches, or generated target output.
- Package file lists are audited by `tools/check-package-files.js`; the
  committed review artifact is [`package-files.json`](package-files.json).
- Publishability blockers are audited by `tools/check-publishability.js`; the
  committed review artifact is [`publishability.json`](publishability.json).
- Internal crate dependencies have explicit versions before any crate is
  published.
- Leaf packages are verified by `just check-package-dry-runs`, which reads
  `design/publishability.json` and runs
  `cargo package --no-verify --allow-dirty` for the generated first publish
  candidates. The current candidates are `rustia-lexer` and `rustia-vm`.
- Full-workspace packaging is still blocked until internal crates are published
  in topological order. The current generated order is `rustia-lexer`,
  `rustia-parser`, `rustia-vm`, `rustia-compiler`, `rustia`, `rustia-cli`,
  then `rustia-conformance`; `rustia-lexer` and `rustia-vm` are the first
  candidates after the policy flip because they have no internal dependencies.
- The benchmark crate must either publish the `rustia-*` crates first, move the
  rustia adapter into a git/path-only integration crate, or ship that adapter as
  an example outside the core package.

## API Gate

- Public error types, source-span/runtime-location accessors, conversion
  traits, retained handles, extension APIs, and hot-reload semantics have stable
  names and documented behavior.
- `Rustia::run<T>()` supports direct typed conversion of single-result scripts.
- Source-named run/compile helpers preserve host-provided source names in
  runtime diagnostics.
- Owned-value conversions cover scalar values, `Option<T>`, `Vec<T>`,
  deterministic `BTreeMap<String, T>`, direct `OwnedTable` values, typed
  owned-table builders, typed string-key lookup, and nested table/array access.
- Capability checks fail closed in deterministic profiles.
- Host functions have documented fuel costs, public fuel-ABI metadata, and
  diagnostic behavior; typed argument conversion errors name the host function
  and argument position.
- Host callbacks can return `rustia::Result<T>` so fallible host errors cross
  the script boundary with host-function diagnostic context.
- Typed host functions and retained script calls support public tuple
  arguments/results through arity 8.
- The public facade has capability-specific helpers for pure functions,
  deterministic input, slot reads/writes, deterministic effects, external
  mutable-state reads, and nondeterministic effects.
- `RustiaExtension` exposes namespace, host-fuel policy, and per-function name,
  fuel ABI version, fuel cost, capability, parameter-count, parameter type,
  return type, and fallibility metadata before installation.
- Public examples cover globals, typed functions, serde, extensions, retained
  functions, and hot reload.
- Owned snapshots expose serde deserialization at the `OwnedValue` level plus
  string-keyed field and array-slot helpers on `OwnedTable`.
- The first public koto-to-rustia migration guide is tracked in
  [`76-koto-migration.md`](76-koto-migration.md).
- Current examples:
  - `crates/rustia/examples/embedding.rs` covers globals, source-named run
    helpers, fallible typed host functions, deterministic extensions, retained
    script functions, and serde.
  - `crates/rustia/examples/hot_reload.rs` covers compile/install hot reload
    plus retained-handle survival and freshness checks.
- Retained `Function` handles and `Exports` expose install generations, and
  failed hot reloads leave the current generation unchanged. `is_current()` /
  `ensure_current()` expose freshness checks, while `Function::call_current()`
  and `Exports::{value,deserialize,get,function}_current()` reject stale
  handles after a successful hot reload.

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
- The dependency-free `bench/site/` dashboard has a checked static asset,
  DOM-binding, JSONL-history, and current-results contract.
- Native baseline history covers the completed adapter matrix on a clean commit.
- Public benchmark claims are stated per workload in `bench/CLAIMS.md`, not as
  a single blanket multiplier. The current generated claim gate approves no
  global speed headline.
- Public benchmark workflows use only the generic `[self-hosted, bench]` runner
  label after owner setup is confirmed.

## Release Process Gate

- `just release-preflight` passes before any publishability flip. It checks
  release notes, package file lists, generated progress HTML, benchmark result
  freshness, crate metadata, workflow policy, example compilation, and the
  workspace test suite.
- `just check-crate-metadata` passes and `design/crate-metadata.json` is
  current.
- `just check-workflows` passes and keeps the hosted CI workflow, manual
  benchmark workflow, action pins, permissions, and generic runner labels in
  their expected public-safe shape.
- `.github/workflows/ci.yml` mirrors the release-preflight gate on hosted Linux
  CI, including example compilation, without running fresh benchmark
  measurements.
- `.github/workflows/benchmarks.yml` provides manual current-matrix measurement
  on the generic `[self-hosted, bench]` runner label and uploads generated
  benchmark artifacts.
- `cargo test --workspace` passes on the release toolchain.
- `just check-examples` passes and compiles the public `rustia` examples.
- `just check-package-files` passes and `design/package-files.json` is current.
- `just check-publishability` passes and `design/publishability.json` is
  current.
- `just check-package-dry-runs` reaches Cargo packaging for the generated
  publish-order leaf candidates from `design/publishability.json`.
- `just check-release-notes` passes and `CHANGELOG.md` names the current
  workspace version plus public-facing unreleased changes.
- `just check-progress`, `just check-project-status`, and
  `just check-benchmark-results` pass without rewriting committed generated
  artifacts or letting README/current-progress facts drift.
- `rustia-cli` supports file, stdin, `--eval`, and `--repl` execution, plus
  profile, fuel, source-name options, line-oriented REPL evaluation, and
  source-aware compile/runtime diagnostics.
- `cargo package --no-verify` reaches packaging for each publishable crate; any
  unpublished path dependency blocker is documented.
- README, `design/CURRENT.md`, `design/progress.json`, generated progress
  HTML, conformance TSVs, benchmark inventory, and current benchmark result
  JSON agree on conformance counts, benchmark state, and release status.
- The crate graph has a documented semver policy before the first non-`0.0.x`
  release.
