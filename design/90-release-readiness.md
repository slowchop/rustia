# Release Readiness Checklist

This checklist tracks what must be true before any public rustia crate flips
from `publish = false` to publishable. Updated 2026-07-29; the ordered runtime
blockers below take precedence over packaging polish.

These gates protect memory safety, lifecycle, compatibility, and the environment contract selected by the
caller. They do not make deterministic execution universal: `standard()` permits explicit nondeterministic
providers/capabilities, and `live()` is a first-class intentionally non-reproducible builder preset. The same
general embedding API must remain available to those users.

## Current Release Blockers (Ordered)

1. Retain broader native/wasm mixed-heap pressure evidence. Maintained heap,
   loaded-program, and VM-control live totals now back quota admission and
   script-visible counters without a whole-VM census at ordinary instruction
   boundaries. Exact reserved telemetry remains on demand, and opt-in peak
   tracking still samples it deliberately. The per-VM logical-live quota,
   atomic/catchable OOM path, VM-owned live/reserved/peak telemetry, epoch-safe
   decision-reset ABI, and focused 600-decision plateau are implemented.
2. Complete the release value and live-handle surface for binary strings,
   buffers, vectors, non-scalar/identity keys, opaque identity-safe userdata
   handles, shared/cyclic table graphs, VM/epoch-safe live `Value`/`Table`/
   `Function` handles, globals/value constructors, a composable chunk builder,
   and a lifetime-safe live serde layer. Preserve the small typed facade as the
   simple layer. Full mlua-style `UserData`/`Scope`, async, and an mlua
   migration shim remain later optional layers unless a concrete release
   consumer promotes them.
3. Harden compiler/runtime error-phase parity, stack traces, source diagnostics,
   transactional extension/module/hot-reload failure behavior, and
   differential/fuzz coverage before stabilizing and publishing the API.
4. Close the full correctness/build/golden target matrix from
   [`00-constraints.md`](00-constraints.md); Linux remains the only active
   timing source, but not the only correctness target.

The cheap worker-local host-context blocker is closed: `HostSchema` registrations
are persistent, each typed invocation borrows a fresh `HostContext`, and bounded
record transactions publish to reusable or caller-owned lanes. Capability-tagged
output/time providers and VM-owned telemetry are also implemented.

The prior module-resolution blocker is also closed: `ServiceModuleResolver`
plugs a host-owned resolver into script-visible `require`; the host returns a
canonical identity plus source, successful modules execute once per decision
epoch, cached values remain GC roots, nested requests receive their referrer's
source name, cycles report the complete identity path, and reset clears guest
module programs/results while preserving the resolver configuration.
`DeterministicSim` admits only a resolver explicitly labeled replay-stable,
while hardened leaves reject dynamic modules. Resolution and module execution
share caller fuel; resolver panics poison until reset; failed or multi-result
modules never enter the cache; and cache admission observes the logical-live
quota. The cache/reset contract is versioned by
`RUSTIA_MODULE_RESOLUTION_ABI_VERSION`; replay/package identity must also bind
the host resolver's content. The CLI's non-deterministic profiles now install a
fixed-order filesystem adapter with canonical identities, referrer-relative
nesting, and module-source runtime diagnostics; deterministic CLI execution has no
ambient loader. Rolling back arbitrary guest global/heap side effects from a
runtime-failing module and supporting a yieldable `require` continuation remain
part of the semantic-tail/failure-semantics blocker.

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
- Every package archive contains `LICENSE-APACHE` and `LICENSE-MIT`; README
  links are either absolute or resolve to files/directories in that archive.
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
- `Rustia<H>` stores a lifetime-independent `HostSchema`; each
  `invoke`/`invoke_into` accepts a fresh `HostContext<H>` whose concrete value
  may borrow block-local input and slots. Typed VMs execute guest code only
  through the lexical `Invocation`.
- Typed callbacks receive only their declared `Input`, `Slots`, `Effects`,
  `SlotWrites`, or profile-gated `HostMut` authority. Callback captures remain
  an explicit semantic trust boundary because safe Rust cannot prove a
  `'static` closure free of global or interior state.
- `InvocationLimits` supplies one shared fuel budget and bounded record-count
  and logical-byte budgets for all guest calls in the invocation. Effects and
  slot writes share one dense sequence.
- Handler failure/panic rolls back that handler's record suffix; an error
  escaping the outer operation publishes no records. Completed host calls
  survive later guest errors caught by `pcall`. Fuel/OOM/record overflow/
  invalid boundary yield/missing typed-host dispatcher/host panic are hard for
  typed transactional execution, without globally changing legacy OOM/yield
  behavior.
- The rollback contract is records only, not guest heap/global mutation,
  profile-gated `HostMut`, callback captures, or host-context interior
  mutation. Host panic poisons the VM; all later guest execution entry points
  reject until decision reset. Containment covers registered callbacks,
  extension ops, stdout/clocks, and the configured `loadstring` compiler.
- `InvocationOutcome` and `InvocationReceipt` expose fuel/count/byte usage;
  `invoke_into` reserves a caller-owned lane before execution and performs no
  allocation or fallible operation in its final append.
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
  - `crates/rustia/examples/typed_invocation.rs` covers `HostSchema`, a
    block-local `HostContext`, capability views, namespaced typed functions,
    shared limits, dense records, usage receipts, and `invoke_into`.
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
- Decision-reset ABI v1 is mechanically tested: configuration/services/
  extensions/installed host bindings/compiled modules/lifetime peaks persist;
  guest programs, globals, upvalues, coroutines, tracebacks, coverage, and
  retained roots are discarded; RNG/logical time/GC scheduling restart; fuel
  remains per execution; stale handles fail by epoch; idle-only rejection and
  a 600-decision native exact post-warmup plateau are covered. The Node/V8
  wasm32 probe covers the same lifecycle and exact plateau across 300 decisions.
- GC pacing is tuned for Standard profiles only after reclamation works, and
  DeterministicSim collection intervals are treated as deterministic ABI.
- Host-provided output/time services and internal VM memory telemetry obey each
  profile's capability policy; placeholder print/time/collectgarbage-count/
  gcinfo behavior is gone, and callbacks can receive cheap worker-local state.
- Transactional typed execution makes fuel, OOM, record overflow, invalid
  boundary yields, missing dispatch, and host panic uncatchable by guest
  `pcall`; record savepoints and outer rollback are tested, and a host panic
  poisons the VM until `reset_decision()`.
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

- `just release-preflight` and hosted CI invoke the same
  `tools/run-release-preflight.js` driver before any publishability flip. It checks
  formatting, strict workspace Clippy, release notes, package file lists,
  generated progress HTML, benchmark result freshness, crate metadata, workflow
  policy, example compilation, and the workspace test suite.
- Hosted CI installs the exact channel from `rust-toolchain.toml` and runs
  `cargo fmt --all -- --check` plus
  locked strict workspace Clippy.
- Every third-party GitHub Action is pinned to a full commit SHA.
- Cargo metadata, Clippy, examples, tests, and package checks use `--locked`;
  preflight rejects a dirty `Cargo.lock` before validation.
- `just check-crate-metadata` passes and `design/crate-metadata.json` is
  current.
- `just check-workflows` passes and keeps hosted CI validation-only: it pins the
  Rust toolchain and actions and forbids workflow-based fresh timing runs.
- `.github/workflows/ci.yml` mirrors the release-preflight gate on hosted Linux
  CI, including example compilation, without running fresh benchmark
  measurements.
- Hosted CI separately cross-compiles the portable `rustia` engine library for
  macOS arm64/x86_64, Linux x86_64/arm64, Windows x86_64 MSVC, iOS arm64,
  Android arm64, and `wasm32-unknown-unknown`. These build checks do not replace
  the still-open real-host and browser correctness/golden gate.
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
