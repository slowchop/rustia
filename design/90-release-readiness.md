# Release Readiness Checklist

This checklist tracks what must be true before any public rustia crate flips
from `publish = false` to publishable. Updated 2026-07-23; the ordered runtime
blockers below take precedence over packaging polish.

## Current Release Blockers (Ordered)

1. Retain native/wasm pressure evidence for the implemented per-VM logical-live
   quota and atomic, catchable OOM path. Host-facing VM-owned live/reserved
   accounting and opt-in instruction-boundary peaks are implemented; an O(1)
   incremental optimization must replace quota-mode boundary scans before
   script-visible compatibility counters can use the same accounting.
2. Add a deterministic decision-boundary reset/reuse API with epoch-safe
   invalidation for every retained handle and a versioned reset/replay ABI.
3. Add cheap worker-local/per-call host context; capability-tagged output and
   clock/time providers plus real internal VM memory telemetry are implemented. Reject
   nondeterministic providers in DeterministicSim.
4. Add a host-owned script-visible `require` resolver/cache with deterministic
   identity, execute-once behavior, and cycle diagnostics.
5. Complete the release value surface for binary strings, buffers, vectors,
   non-scalar/identity keys, opaque identity-safe userdata handles,
   shared/cyclic table graphs, and a lifetime-safe live serde layer. Full
   mlua-style `UserData`/`Scope` registration remains a later optional layer.
6. Harden compiler/runtime error-phase parity, stack traces, source diagnostics,
   transactional extension/module/hot-reload failure behavior, and
   differential/fuzz coverage before stabilizing and publishing the API.
7. Close the full correctness/build/golden target matrix from
   [`00-constraints.md`](00-constraints.md); Linux remains the only active
   timing source, but not the only correctness target.

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
- Treat no unsafe code as a public-workspace feature: every workspace package
  must inherit the workspace `unsafe_code = "forbid"` lint. Experimental
  spikes and standalone wasm FFI demos are not part of the publishable product
  gate.

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
- Complete reachability and slot reuse reclaim unreachable ordinary VM objects
  and loaded programs without allowing stale handles to alias reused slots.
- Live/peak allocation accounting, aggregate quotas, and atomic OOM behavior
  are covered under long-running native and wasm churn.
- A decision reset defines and tests exactly which programs, globals, upvalues,
  coroutines, RNG state, fuel, and collection counters reset or persist.
- GC pacing is tuned for Standard profiles only after reclamation works, and
  DeterministicSim collection intervals are treated as deterministic ABI.
- Host-provided output/time services and internal VM memory telemetry obey each
  profile's capability policy; placeholder print/time/collectgarbage-count/
  gcinfo behavior is gone, and callbacks can receive cheap worker-local state.
- Module resolution/cache, rich value interop, error-phase parity, and public
  stack/source diagnostics satisfy their focused tests; failed extension/module
  installation and runtime-failing reload cannot leave undocumented partial state.
- The value-layout and dispatch wasm pre-lock gate is closed with retained V8,
  JavaScriptCore-family, and SpiderMonkey evidence; recurring Node/V8 evidence
  remains current.
- Native and browser correctness/build/golden coverage reaches the full
  `design/00-constraints.md` matrix before release, independently of the
  dedicated-Linux-only benchmark policy.

## Benchmark And CI Gate

- `bench/history/runs.jsonl`, `bench/RESULTS.md`,
  `bench/current-results.json`, `bench/current-results.html`, and
  `bench/CLAIMS.md` are generated from the same committed benchmark history.
- `bench/current-results.html` is the single checked benchmark report artifact.
- Native baseline history covers the completed adapter matrix on a clean commit.
- Public benchmark claims are stated per workload in `bench/CLAIMS.md`, not as
  a single blanket multiplier. The current generated claim gate approves no
  global speed headline.
- Fresh performance data comes only from the explicit ephemeral Linode
  Linux/x86_64 lifecycle. macOS and Windows remain build targets, not active
  benchmark targets, until dedicated hosts exist.

## Release Process Gate

- `just release-preflight` passes before any publishability flip. It checks
  formatting, strict workspace Clippy, release notes, package file lists,
  generated progress HTML, benchmark result freshness, crate metadata, workflow
  policy, example compilation, and the workspace test suite.
- Hosted CI installs the exact channel from `rust-toolchain.toml` and runs
  `cargo fmt --all -- --check` plus
  `cargo clippy --workspace --all-targets -- -D warnings`.
- `just check-crate-metadata` passes and `design/crate-metadata.json` is
  current.
- `just check-workflows` passes and keeps hosted CI validation-only: it pins the
  Rust toolchain and actions and forbids workflow-based fresh timing runs.
- `.github/workflows/ci.yml` mirrors the release-preflight gate on hosted Linux
  CI, including example compilation, without running fresh benchmark
  measurements.
- The explicit Linode matrix command provisions, measures, validates, deletes,
  and only then promotes a coherent native-plus-wasm Linux result set. Hosted
  workflows do not provision billed resources or publish timings.
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
  artifacts, skipping the fixed-sample analyzer self-test or wasm gate verifier,
  or letting README/current-progress facts drift.
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
