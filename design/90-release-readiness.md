# Release Readiness Checklist

This checklist tracks what must be true before any public rustia crate flips
from `publish = false` to publishable. Updated 2026-09-02; the ordered runtime
blockers below take precedence over packaging polish.

These gates protect memory safety, lifecycle, compatibility, and the environment contract selected by the
caller. They do not make deterministic execution universal: `standard()` permits explicit nondeterministic
providers/capabilities, and `live()` is a first-class intentionally non-reproducible builder preset. The same
general embedding API must remain available to those users.

## Current Release Blockers (Ordered)

The mixed-heap correctness gate is closed by the shared native/Node-V8
`mixed_heap_pressure` test: 96 epochs and 3,072 waves combine bounded public
roots, cyclic tables, strings, closures, suspended coroutines, userdata,
buffers, vectors, weak tables, hot reload, quota/OOM recovery, and decision
reset under a 2 MiB logical-live limit. Native and wasm runs hold the same
semantic invariants and exact post-reset VM plateau; warmed wasm linear-memory
pages also remain constant. This does not claim equal process RSS or allocator
overhead across targets.

1. Complete the release value ergonomics on the implemented VM/epoch-safe,
   graph-preserving rooted `Value`/`Table`/`Function` spine. It already covers
   binary strings, buffers, vectors, non-scalar/identity keys, shared/cyclic
   tables, mutation, raw equality, and ordinary or typed invocation exchange.
   Live globals, quota-aware tables, owned-key/live-value insertion, and the
   composable chunk builder, snapshot-free live serde, and reclaimable synchronous host functions
   are implemented. Complete the controlled-client migration and define a concrete opaque-userdata
   registration contract. Preserve the
   small typed facade as the simple layer. Full mlua-style `UserData`/`Scope`,
   async, and an mlua migration shim remain later optional layers unless a
   concrete release consumer promotes them.
2. Harden compiler/runtime error-phase parity, stack traces, source diagnostics,
   yieldable `require` and
   differential/coverage-guided fuzz evidence before stabilizing and publishing
   the API. The stable bounded baseline already mutates 2,048 frontend cases
   and 2,048 VM byte streams while covering all 131 production opcodes. Exact
   f32/f64 buffer-bit round trips, including NaN payloads, are covered.
3. Close the full correctness/build/golden target matrix from
   [`00-constraints.md`](00-constraints.md); Linux remains the only active
   timing source, but not the only correctness target.

The cheap worker-local host-context blocker is closed: `HostSchema` registrations
are persistent, each typed invocation borrows a fresh `HostContext`, and bounded
record transactions publish to reusable or caller-owned lanes. Capability-tagged
output/time providers and VM-owned telemetry are also implemented.

The module-resolution and installation-failure blocker is also closed: `ServiceModuleResolver`
plugs a host-owned resolver into script-visible `require`; the host returns a
canonical identity plus source. Ordinary execution has one decision-wide
execute-once cache, while each installation receives a fresh lazy cohort shared
by its dependency graph and selected by executing function origin. Cached
values remain GC roots while their cohort is reachable, nested requests receive
their referrer's source name, cycles report the complete identity path, and
reset clears guest module programs/results while preserving resolver configuration.
`DeterministicSim` admits only a resolver explicitly labeled replay-stable,
while hardened leaves reject dynamic modules. Resolution and module execution
share caller fuel; resolver panics poison until reset; failed or multi-result
modules never enter the cache; and cache admission observes the logical-live
quota. The cache/reset contract is versioned by
`RUSTIA_MODULE_RESOLUTION_ABI_VERSION`; replay/package identity must also bind
the host resolver's content. The CLI's non-deterministic profiles now install a
fixed-order filesystem adapter with canonical identities, referrer-relative
nesting, and module-source runtime diagnostics; deterministic CLI execution has no
ambient loader. Atomic `install` gives every candidate a fresh inheriting
environment and fresh lazy dependency cohort, rolls back guest heap/global/
coroutine/RNG/logical-time/GC/cache state and provisional roots on failure, and
is versioned by `RUSTIA_INSTALL_TRANSACTION_ABI_VERSION`. External host and
resolver effects remain host-owned. Supporting a yieldable `require`
continuation remains part of the semantic-tail blocker.

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
  traits, retained handles, extension APIs, and replacement semantics have stable
  names and documented behavior.
- `Rustia::eval<T>()` supports direct typed conversion of single-result scripts.
- `load(...).source_name(...)` preserves host-provided source names in runtime
  diagnostics for every terminal operation.
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
- The general invocation rollback contract is records only, not guest
  heap/global mutation, profile-gated `HostMut`, callback captures, or
  host-context interior mutation. `Invocation::install` adds a nested
  guest-state savepoint whose success stays provisional until the outer record
  transaction commits. Host panic poisons the VM; all later guest execution entry points
  reject until decision reset. Containment covers registered callbacks,
  extension ops, stdout/clocks, and the configured `loadstring` compiler.
- `InvocationOutcome` and `InvocationReceipt` expose fuel/count/byte usage;
  `invoke_into` reserves a caller-owned lane before execution and performs no
  allocation or fallible operation in its final append.
- `RustiaExtension` exposes namespace, host-fuel policy, and per-function name,
  fuel ABI version, fuel cost, capability, parameter-count, parameter type,
  return type, and fallibility metadata before installation.
- Public examples cover globals, typed functions, serde, extensions, retained
  functions, and transactional replacement.
- Owned snapshots expose serde deserialization at the `OwnedValue` level plus
  string-keyed field and array-slot helpers on `OwnedTable`.
- Rooted live `Value`, `Table`, and `Function` handles preserve VM identity and
  decision-epoch safety across binary strings, buffers, vectors, rich keys,
  shared/cyclic tables, mutation, raw equality, and ordinary or typed
  invocation exchange. Snapshot conversion stays explicit and reports cycles.
- The first public koto-to-rustia migration guide is tracked in
  [`76-koto-migration.md`](76-koto-migration.md).
- Current examples:
  - `crates/rustia/examples/embedding.rs` covers globals, source-named execution
    helpers, fallible typed host functions, deterministic extensions, retained
    script functions, and serde.
  - `crates/rustia/examples/hot_reload.rs` covers atomic candidate installation,
    host-owned replacement, and old retained-handle survival.
  - `crates/rustia/examples/typed_invocation.rs` covers `HostSchema`, a
    block-local `HostContext`, capability views, namespaced typed functions,
    shared limits, dense records, usage receipts, and `invoke_into`.
  - `crates/rustia/examples/live_values.rs` covers graph-preserving live table
    access, mutation, raw identity, and live function arguments/results.
- Retained `Function` handles and `Exports` are checked by VM identity and
  decision epoch. Successful unrelated installations do not stale them; old
  functions retain their originating dependency cohort and remain callable
  until decision reset.

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
  a 600-decision native exact post-warmup plateau are covered. The shared
  mixed-heap lifecycle adds 96 native/Node-V8 epochs with exact post-reset VM
  accounting and stable warmed wasm linear-memory pages.
- GC pacing is tuned for Standard profiles only after reclamation works, and
  DeterministicSim collection intervals are treated as deterministic ABI.
- Host-provided output/time services and internal VM memory telemetry obey each
  profile's capability policy; placeholder print/time/collectgarbage-count/
  gcinfo behavior is gone, and callbacks can receive cheap worker-local state.
- Transactional typed execution makes fuel, OOM, record overflow, invalid
  boundary yields, missing dispatch, and host panic uncatchable by guest
  `pcall`; record savepoints and outer rollback are tested, and a host panic
  poisons the VM until `reset_decision()`.
- Module resolution/cache, installation rollback, the remaining live-value
  ergonomics/serde, error-phase parity, and public stack/source diagnostics
  satisfy their focused tests; failed extension/module installation cannot
  leave guest partial state.
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
- Fresh performance data comes only from the owned benchmark host lanes
  (`ci-bench`, Linux/x86_64). macOS and Windows remain build targets, not active
  benchmark targets, until dedicated hosts exist.

## Release Process Gate

- `just release-preflight` and hosted CI invoke the same
  `tools/run-release-preflight.js` driver before any publishability flip. It checks
  formatting, strict workspace Clippy, strict Clippy for the builds only the
  benchmark host compiles, release notes, package file lists, generated progress
  HTML, benchmark result freshness, crate metadata, workflow policy, example
  compilation, and the workspace test suite.
- Hosted CI installs the exact channel from `rust-toolchain.toml` and runs
  `cargo fmt --all -- --check` plus
  locked strict workspace Clippy.
- Every third-party GitHub Action is pinned to a full commit SHA.
- Cargo metadata, Clippy, examples, tests, and package checks use `--locked`;
  preflight rejects a dirty `Cargo.lock` before validation.
- Preflight runs `cargo fetch --locked` against the manifest beside every
  tracked `Cargo.lock`: the root, `fuzz/`, and the standalone wasm runners in
  `bench/wasm/adapter-runner/` and `demos/live-wasm/wasm-runner/`. The
  owned-host benchmark lanes build the adapter with `--locked`, so a lock left
  stale by a shared-crate dependency change fails here instead of on the host.
- Preflight runs Clippy with `-D warnings` on the three builds that otherwise
  compile only on the owned benchmark host, for the weekly matrix and the
  memory lanes: `bench` with `--no-default-features --features all-engines`
  (every competitor engine), `bench` with `--no-default-features --features
  mlua-lua54` (the one build with `rustia-engine` off), and the standalone
  adapter in `bench/wasm/adapter-runner/` for `wasm32-unknown-unknown`. The
  workspace Clippy reaches none of their cfg paths. Both host workflows
  inherit the pinned toolchain action's default `RUSTFLAGS=-D warnings`, so a
  plain `cargo check` would let through a warning that fails the host build.
  The checks catch compile errors and warnings. Link and codegen failures
  still surface only on the host. They add about 0.4 s to a warm preflight
  with nothing changed, about 2.3 s after an engine-crate edit, and about
  51 s from cold, measured on an M4 Max while other builds were running.
  The CI preflight job installs the wasm target itself, because the action's
  `rustup override` bypasses the target list in `rust-toolchain.toml`.
- `just check-crate-metadata` passes and `design/crate-metadata.json` is
  current.
- `just check-workflows` passes and keeps hosted CI validation-only: it pins the
  Rust toolchain and actions and forbids workflow-based fresh timing runs.
- `.github/workflows/ci.yml` mirrors the release-preflight gate on hosted Linux
  CI, including example compilation, without running fresh benchmark
  measurements.
- Hosted CI separately cross-compiles the portable `rustia` engine library for
  macOS arm64/x86_64, Linux x86_64/arm64, Windows x86_64 MSVC, iOS arm64,
  Android arm64, and `wasm32-unknown-unknown`. It also executes the bare and
  default web-live wasm adapters through the matching wasm-bindgen test runner
  on Node 22/V8. These checks do not replace the still-open non-Linux real-host
  and Firefox/Safari correctness/golden gate.
- Release preflight formats and strictly lints the separate fuzz workspace,
  then runs its seed tests plus deterministic 2,048-case frontend and VM
  mutation sweeps. The VM decoder covers every production opcode exactly once
  under bounded input, fuel, recursion, and a 512 KiB logical-live limit.
- The cross-engine matrix workflow measures on the owned benchmark host,
  validates, and only then promotes a coherent single-revision native-plus-wasm
  Linux result set; it lands that cohort on `main` through the repository's
  single sanctioned CI contents-write grant. No workflow can provision a billed
  resource.
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
