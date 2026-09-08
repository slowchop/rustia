# Changelog

All notable public-facing changes to rustia will be recorded here.

Current unpublished workspace version: `0.0.1`.

## [Unreleased]

### Added

- Human-facing `Error::report()` with immutable executed-source snapshots,
  expression spans, bounded logical callers, factual operation explanations,
  parser-proven unclosed-block notes, typed host causes, serde field paths, and
  explicit installation rollback context. Configurable host diagnostic limits
  remain independent of guest fuel/heap quotas; hardened leaves do not retain
  rich diagnostic source or frames. Console prefix/wrapper mapping is available
  on ordinary and typed chunk builders. The CLI uses the shared renderer.
- Pre-release error API cleanup: `Error` is a typed object with `ErrorKind`,
  `as_vm_error`, `embed`, `external`, and classification-preserving context.
  Module resolvers return `rustia::Result<ModuleSource>` instead of flattening
  errors into strings. Guest error strings and protected-call behavior are unchanged.

- Synchronous `HostFunction` definitions shared by `create_function`,
  `register_function`, and `HostModule::function`. Typed signatures support
  owned and live arguments and mixed tuple results through arity eight.
  Constructed callbacks follow rooted guest-function lifetime and are reclaimed
  after their last reachable reference; reset invalidates them and releases
  their captures. Registrations remain reset-persistent and replacing one
  releases obsolete configuration ownership without breaking guest aliases.
  Capability views, fuel, signature metadata, quota admission, and panic
  containment are shared. Persistent registration requires an idle VM;
  hardened leaves reject disposable construction before VM allocation.

- Transactional module installation through `load(...).install()` and
  `install(&CompiledModule)`. Each candidate runs in a fresh globals-inheriting
  environment and fresh lazy `require` cache cohort unless the host supplies an
  exact environment. Compile, runtime, quota, cache-admission, and retained-root
  failures restore guest heap/global/coroutine/RNG/logical-time/GC/cache state;
  old exports remain callable, so the host performs replacement with ordinary
  Rust assignment. Typed installation is a nested savepoint whose records stay
  provisional until the enclosing invocation commits. The rollback snapshot is
  charged to the VM memory quota before candidate execution begins. The contract is
  versioned by `RUSTIA_INSTALL_TRANSACTION_ABI_VERSION`.
- Atomic global and namespaced extension installation. A quota or validation
  failure cannot leave an earlier function from the same batch installed.

- Snapshot-free Serde deserialization from rooted `Value`, `Table`, and `Exports` handles. The direct path
  preserves VM/decision-epoch ownership, rejects cycles and unsupported
  rich values without exposing collector lifetimes, and works with typed `Rustia<H>` VMs. The existing owned
  serializer and explicit eager snapshot path remain available.

- A composable `Rustia::load` chunk builder with borrowed text/byte source,
  source names used for diagnostics and `require` resolver referrers, rooted custom lexical environments, and
  `eval`/`eval_live`/`eval_serde`/`exec`/`install` terminals. Custom environments are checked
  for VM and decision-epoch ownership before parsing or compilation, functions
  created by a chunk capture the selected live table, and hardened leaves
  reject dynamic loading before compilation or VM allocation.

- Pull-request benchmark screening on the owned host.
  `.github/workflows/pr-bench.yml` measures a branch against
  `git merge-base origin/main HEAD` — not main's tip, which would attribute an
  unrelated commit landing mid-review to the change — over two passes with the
  order reversed in the second, so the two sides interleave A B B A. It reports
  in the job summary and records nothing: no row, no published artifact, and no
  write access to either repository, each pinned by `checkPrBench`. Fork pull
  requests never reach the benchmark host; that guard is in place now rather
  than deferred to whenever the repository goes public.
- Measured rather than declared screening thresholds. `just bench-calibrate`
  measures one revision four times and compares it against itself under all
  three balanced splits of those passes, so every interval is the host's own
  spread. A workload's threshold is the widest end of the widest split. There is
  no default and no fallback: a declared figure that happens to sit inside the
  host's spread would report movement on changes that did nothing. The A/A run
  is also the protocol's canary — an interval that excludes zero when nothing
  changed means the comparison is measuring something other than the code, and
  the calibration refuses to write itself rather than let a wider threshold
  paper over it.
- `comparison.direction` in the fixed-sample analyzer, stating faster, slower or
  indistinguishable against zero. The material-regression threshold is
  one-sided, so a genuine speedup previously reported
  `material_regression_excluded` — true, and it reads as if nothing happened.
  Replaying the committed `84e732a` analysis shows an 8.2% improvement published
  under exactly that wording.
- `--confidence` in the analyzer. Screening five workloads at 95% gives roughly a
  23% chance one of them moves on a change that did nothing, and more
  repetitions cannot fix a per-test error rate, so the level had to become a
  parameter. It is refused unless the run has at least ten bootstrap resamples
  per tail. The `*_ci_95_percent` fields stay at 95% whatever is asked for, so
  their names remain true and consumers written before the option read what they
  always read.
- `tools/record-host-bridge.js`, which records how two benchmark machines
  compared before rows measured on one of them are deleted. Published figures are
  ratios against the reference measured in the same run, so a change of host
  cancels there; absolute nanoseconds do not.
- `tools/retire-cloud-history.js`, which removes cohorts measured on retired
  hosts only if every generated report comes out byte-identical, and restores the
  file if any of them moves.

- Automated cross-engine matrix benchmarking on the owned host:
  `.github/workflows/weekly-matrix-bench.yml` measures all thirteen engines
  across the five workloads on `ci-bench` and pushes the accepted cohort to
  `main`, so the published history and dashboard update without operator
  action. It replaces a nightly job that created a billed cloud instance and
  ran only when performance-relevant commits had landed; that gate asked the
  Rustia-progress question, which the per-commit series now answers directly,
  and running unconditionally makes host drift visible rather than hiding it.
  Dispatch-only until the first owned-host cohort has been inspected. The
  workflow shape — including the repository's only CI contents-write grant —
  is pinned by `tools/check-workflows.js`.

- VM Core 2 C2.2 replaces the plain-mode per-PC Region/receipt selection
  ladder with one loader-built, whole-function executable tape. Every source
  PC predecodes to exactly one private tape operation — register moves and
  cell clears, primitive constant loads with the value decoded at load time,
  guarded numeric arithmetic/negation/comparisons, truthiness inversion,
  guarded dense-array reads and writes, direct `Jump`/`JumpIf`/`JumpIfNot`
  branch targets, guarded numeric `ForLoop` backedges, and the accepted fused
  four/six-operation register receipts — or to an effect boundary owned by the
  scalar dispatcher. Execution mode (plain, metered, instrumented) is selected
  once at function entry: unmetered, uninstrumented frames run the tape in
  effect-terminated blocks whose run suffix doubles as the logical-cost prefix
  for deterministic-GC admission, while metered and instrumented frames stay
  entirely scalar. A dynamic guard miss side-exits to the exact canonical
  instruction before any mutation, malformed operands decline at build time so
  structured errors keep their kind and PC, HotIR-owned `ForLoop` backedges
  stay effect boundaries, and tape metadata is charged exactly to
  loaded-program quota admission. The Region suffix map and per-PC
  RegisterBlock map are retired; the receipt recognizer and executor survive
  inside the tape. Scalar-versus-tape differential tests sweep every tape
  opcode family, deterministic collection intervals, guard exits, branch
  edges, malformed jump targets, and error PCs. The accepted controlled
  DigitalOcean gate measured 4.0059% lower candidate `generic_iteration`
  ns/unit against the receipt baseline (pooled medians 13,092.320 versus
  12,567.856), with a conservative nominal 95% interval from -7.1417% to
  -2.8710%. All 808 formal samples matched the checksum, all 19 timed legs
  observed zero steal, and the Droplet/key cleanup gate passed. Evidence is
  retained under
  `bench/history/evidence/77e0d92871133277340b03b2eda119974babecda/`.

- VM Core 2 C2.2 adds frame-local immutable execution receipts over unchanged
  public bytecode. Region Engine can execute the compiler's exact
  `Move`/`Move`/numeric-or-dense-index/`ClearLocalCell` shape as one internal
  operation while retaining the surrounding uninterrupted Region. An exact
  six-operation form also absorbs the compiler's destination commit and result
  cleanup for hot uncaptured-local updates. Receipt metadata is resolved once
  at function entry; success advances four or six logical instructions, while
  a guard miss runs the canonical operations in order.
  Fuel, coverage, deterministic-GC cadence, structured error PCs, scalar
  metamethod fallback, and exact loaded-program quota admission remain pinned.
  This replaces the C2.1 peer RegisterBlock/ClearSpan tier: its controlled
  DigitalOcean gate measured a 5.293% slowdown with a conservative nominal 95%
  interval of +3.720% to +6.135%, so publication was rejected, ClearSpan was
  removed as unexonerated, and public benchmark history was not changed. The
  compiler now pins both the four-operation prefix and six-operation lifetime
  shape. The accepted controlled C2.2 DigitalOcean gate then measured 8.1651%
  lower candidate `generic_iteration` ns/unit, with a conservative nominal 95%
  interval from -10.2126% to -5.9577%. All 808 formal samples matched the
  checksum, three final-binary measurements were retained as sidecars, and the
  Droplet/key cleanup gate passed. Evidence is retained under
  `bench/history/evidence/84e732add4863150bd993f3354b274ab9c3ee41a/`.

- Region Engine v1 precomputes private side metadata for consecutive,
  effect-certified primitive-constant, register-bookkeeping, guarded-numeric,
  and guarded dense-array instructions. One region admission amortizes the
  interpreter's repeated policy and dispatch checks while keeping public
  bytecode authoritative. Dynamic guard misses resume at the exact failing
  instruction, and fuel, coverage, allocation-debt, and deterministic-GC
  boundaries retain the ordinary interpreter path. Focused equivalence tests
  cover successful regions, guard exits and source locations, fuel and
  coverage fallback, deterministic collection cadence, and coexistence with
  HotIR. The accepted controlled DigitalOcean A/B measured +1.3181% candidate
  ns/unit with a conservative 95% interval from -4.5763% to +3.9306%. That
  excludes a material 5% regression but does not establish a speedup; evidence
  is retained under
  `bench/history/evidence/b861ee998ebb986b662cd6249e61169f86d685ac/`.

- Benchmark operations now have a versioned policy, generated machine-readable
  freshness status, and a prominent generated-dashboard notice. The status
  records pending Rustia performance commits while explicitly marking current-
  engine landscape refreshes and artifact-only publication as paused. Shared,
  CPU-busy self-hosted Actions runners remain available for builds,
  correctness, validation, relevance detection, and publication, but are
  explicitly non-authoritative for timing and cannot schedule billed hosts.
  The validation-only benchmark-data publisher now uses that reviewed
  `general-linux` pool instead of budget-blocked GitHub-hosted capacity,
  disables checkout credentials, and removes its temporary deploy key and
  worktrees on every exit before the persistent runner can be reused.

- A new `Profile::Puc51Compat` PUC Lua 5.1.5 compatibility profile: a faithful
  `ltable.c` table backend (array part + node part with main-position chaining,
  Brent-style relocation, `computesizes` rehash, tombstone deletion, `luaH_next`
  enumeration order, and `luaH_getn` border), the PUC `auxsort` for
  `table.sort`, `%.14g` number formatting through `tostring`/concat/
  `table.concat`/`string.format("%s")`, constructor pre-sizing (compiler
  hash-field hints plus `luaH_resizearray` append tails), and PUC constructor
  write order (immediate keyed writes vs `LFIELDS_PER_FLUSH` positional
  batches, via `CompileOptions::puc51_constructor_order`). The `rustia` facade
  gains a `RustiaBuilder::puc51_compat()` preset pairing the profile with the
  stock `loadstring` compiler, and the CLI accepts `--profile puc51-compat`.
  Behavior parity is pinned by a new offline-derived differential corpus under
  `conformance/fixtures/puc51-compat/` (18 fixtures; expectations captured from
  the pinned PUC 5.1.5 C sources in the new `third_party/lua-src` submodule —
  everything rustia builds stays pure Rust; the C oracle is compiled only
  offline and never by CI). Accepted divergences are documented in
  `design/50-conformance.md`.

### Changed

- Low-level VM host functions now own their callback operations in generation-safe
  heap slots. `FunctionKind::Host` no longer exposes a registry index, and the
  contextual dispatcher receives owned type-erased handler data instead of an
  append-only numeric key. `HostDispatcherUnavailable` no longer contains a key.
  Captured destructors must not panic; opaque Rust captures remain outside VM quota.
- The unpublished `0.0.x` facade now uses one coherent execution vocabulary:
  `eval`, `eval_live`, `eval_serde`, `exec`, and `install`, configured through
  `load(...).source_name(...).environment(...)`. The former `run*`, named
  run/compile variants, `eval_value`, `eval_exports`, and `hot_reload` methods
  are removed. `ModuleSource::named` becomes `with_source_name`.
- Install-generation/currentness methods are removed from `Rustia`, `Value`,
  `Table`, `Function`, and `Exports`. Retained handles are invalidated by a
  different VM or decision reset, not by a successful unrelated installation.
  Functions retain their originating lazy dependency cohort.

- Serde sequence requests now reject sparse or mixed map-shaped tables instead of silently producing an empty
  sequence, and numeric map keys use the same finite, integral, range-checked conversions as numeric values.

- **Nothing in this repository can create or pay for a cloud instance.** The
  memory cohort — the last lane that could, and only ever by explicit dispatch —
  now runs on the owned host through `.github/workflows/rustia-memory.yml`. Its
  protocol pins two isolated cores rather than one, because the supervisor
  samples while the benchmark runs and sharing a core would make the supervisor
  part of what it measures. The eleven Linode lifecycle recipes are removed.
  `tools/run-linode-benchmark.js` remains only as the home of the matrix and
  memory acceptance rules that the owned-host coordinator uses; it is documented
  as retired and nothing invokes it.
- Owned-host memory evidence no longer carries filenames naming a provider it
  never ran on: `linode-build.json` and `linode-run-gate.json` become
  `build.json` and `run-gate.json` for that lifecycle. The validator accepts both,
  so committed evidence from the retired lane keeps validating.

- **Removed 1,230 cross-engine rows measured on retired cloud hosts** from
  `bench/history/runs.jsonl`, leaving the single cohort measured on the owned
  host. This is a destructive change to a published dataset and is recorded as
  its own entry for that reason. It is safe because the generated reports only
  ever showed the newest row per engine/workload slot, and that row already
  belonged to the kept cohort: `tools/retire-cloud-history.js` regenerates
  `current-results.json`, `RESULTS.md`, `CLAIMS.md` and the dashboard afterwards
  and refuses to proceed unless every byte is identical. The removed rows remain
  in this repository's Git history and in the published data repository's.
- Before that deletion, `bench/evidence/matrix/<cohort>/host-bridge.json` records
  how the two machines compared, per engine and per workload. That comparison
  could not have been recovered afterwards, and it is not a constant: the ratio
  between the owned host and the retired instance ranges from 0.71 to 1.86
  depending on the engine — `roto` reads 1.75x slower on the new machine and
  `wasmtime` 0.82x — so a reader looking at absolute nanoseconds across the
  change would otherwise see engine movements that never happened. Published
  figures are ratios against the reference measured in the same run, so they are
  unaffected.

- Every benchmark now runs on one owned, attested machine. The DigitalOcean,
  Vultr and Scaleway coordinators are deleted, and the cross-engine matrix moved
  from a nightly billed cloud instance to `.github/workflows/weekly-matrix-bench.yml`
  on that host. Nothing in this repository can create, authenticate to, or pay
  for a cloud instance. The matrix also lost its pending-commits gate: that gate
  asked the Rustia-progress question, which the per-commit series now answers
  directly, and running unconditionally makes host drift visible rather than
  hiding it.
- Frozen configs behind published measurements are retained even though their
  coordinators are gone; they are the protocol a reader needs to check a number
  still on the dashboard. Configs no committed evidence referenced were deleted
  with their coordinator. Roughly two hundred lines of cloud operating
  instructions were removed, and with them a developer machine's host alias,
  which should not have been in a public-bound repository.
- The workflow checker's ban on the raw benchmark harness now names the timing
  rather than the binary. `--refresh-results` returns before any measurement and
  only re-derives the report tables from committed history; banning the command
  outright is why the retired matrix lane could reach it through `just` for
  months without any rule noticing.

- DigitalOcean protocol v6 now evaluates paired-host stability within matching
  AB/BA orders. This retains the four-pair counterbalance while preventing a
  repeatable second-leg frequency effect from being mistaken for random host
  instability; raw cross-order spread remains recorded for audit. The 5%
  stability ceiling, 0.5% timed-steal ceiling, and independent eight-leg
  acceptance test are unchanged.

- Fixed-work optimization acceptance now promotes evidence without appending a
  partial single-workload cohort to `bench/history/runs.jsonl`. The public
  dashboard gains an accepted-optimization table for the exact A/B delta,
  conservative confidence interval, verdict, and sample count while preserving
  the latest complete cross-engine matrix.

- DigitalOcean's paired acceptance protocol now rejects an irrevocably unstable
  host as soon as the observed ratio spread exceeds its frozen limit. Protocol
  v5 retains zero-steal idle admission and caps the paired-ratio gate at the
  predeclared 5% material-regression boundary. It bounds timed steal at 0.5%,
  one tenth of that threshold, because this account exposes only shared-CPU shapes.
  Rejected qualification samples remain diagnostic-only and are never reused.

- The interpreter's common-instruction fast path now also serves `Move` and
  `ClearLocalCell` — pure register bookkeeping that provably cannot allocate,
  invoke guest code, or trigger collection, so the slow path's GC-root
  refresh, fuel charging, and collection polling are skipped — and the
  candidacy predicate is a single indexed table instead of a compare chain.
  A dynamic opcode histogram showed the two opcodes at 42-61% of executed
  instructions on four tracked workloads. A controlled dedicated-host A/B
  (baseline `6d22fa3`, `generic_iteration`, fixed `n=4000`, interleaved
  ABBA/BAAB) measured -8.45% candidate median ns/unit with a conservative 95%
  interval of -9.06% to -7.35%; evidence under
  `bench/history/evidence/f412e2dadd4568b4ba35158090978c79413fd0f7/`. The
  single-workload rows stay out of public history per the dedicated-cohort
  policy and land in the published results with the next full-matrix cohort.

- `LoadStringCompiler` now receives the VM's `Profile` as a third parameter so
  runtime-compiled chunks honor profile-dependent codegen (currently the PUC
  constructor write order under `Profile::Puc51Compat`). Update custom
  loadstring compilers to the new signature.

### Fixed

- Quiescent spare-capacity release now bumps a table's mutation version when
  it pops trailing array nils: the pop moves the observable length border,
  and version consumers (the `table.sort` mutation guard) previously saw no
  mutation. A
  regression test pins the border movement and the version bump.

- The exact-commit `34f6deff4a42501b6d66d257249eecfcbca1976a` cross-engine
  Linode matrix replaced the public Linux snapshot (65 native rows, 15 wasm
  rows) with all four timed groups accepted on their first timed attempt and
  zero timed steal. Versus the preceding `83bc2f3` snapshot — 86 commits
  earlier, so movements are stability signals rather than single-change
  attribution — rustia median ns/unit moved -7.07% dense, +8.06% sparse,
  +20.97% closures, +16.10% generic iteration, and +17.32% stdlib, and the
  generated minimum-time `mlua-luau` geometric-mean gap is 3.944x. The VM was
  confirmed deleted before evidence and results promotion.

- A mutation-version-keyed cached table length-border slice was implemented
  and exactly gated, then reverted after the accepted protocol falsified its
  premise: the dense_tables Linode A/B measured +1.7569% candidate ns/unit
  (conservative nominal 95% interval +0.02% to +3.63% — regression excluded,
  no win) and the sparse_tables A/B measured +0.6158% (interval -4.25% to
  +2.03% — neutral). Recorded as falsified-bet evidence in
  `design/56-performance-research.md`, not as a speed claim; only the
  spare-capacity mutation-version fix above survives.

- A per-call-site generalized-iterator leaf-step cache slice was implemented
  and exactly gated, then reverted after the accepted protocol falsified its
  premise: the generic_iteration Linode A/B measured +3.3591% candidate ns/unit
  (conservative nominal 95% interval +1.58% to +4.87% — the predeclared +5%
  material regression excluded, but zero and any win also excluded). Recorded
  as falsified-bet evidence in `design/56-performance-research.md`, not as a
  speed claim.

- The fixed-run Linode promotion validator now accepts the harness's current
  24-key history row schema instead of the stale 13-key shape that previously
  forced an automatic promotion rollback after an accepted remote run.
- History era splitting in the results renderer now tries the widest gaps
  first and skips any split whose newer side lacks full five-workload
  coverage, so a partial tail cohort can no longer poison the rendered
  current-results report; output is byte-identical on existing history.

- One-argument `table.create(n)` under `Profile::Puc51Compat` no longer
  collides with compiler constructor pre-sizing hints: constructor
  `TableCreate` instructions now mark with `aux == 0` (the builtin's arity is
  always 1..=2), so the builtin's argument is no longer misread as a
  hash-field hint (spurious "type error: expected number", node-part
  over-allocation from unrelated register data, and a bypassed builtin size
  cap).

- A shared native/Node-V8 mixed-heap pressure test now executes 96 decision
  epochs and 3,072 allocation waves under a 2 MiB logical-live limit. Each
  wave combines cyclic tables, runtime strings, closures/upvalues, suspended
  coroutines, userdata, buffers, vectors, weak tables, and bounded public
  roots; each epoch also exercises hot reload, an atomic catchable 8 MiB
  allocation failure, weak-root cleanup, stale-handle rejection, and an exact
  reset plateau. The wasm run additionally requires linear-memory pages to
  stop growing after the warm half.
- The stable-toolchain fuzz smoke gate now performs deterministic 2,048-case
  frontend and 2,048-case VM mutation sweeps. The VM decoder covers every one
  of the 131 production opcodes exactly once under bounded input,
  instruction fuel, recursion, and a 512 KiB logical-live quota; the separate
  `cargo-fuzz` targets remain available for longer nightly
  coverage-guided campaigns.
- Hosted CI now executes both bare/no-default-feature and default web-live
  `wasm32-unknown-unknown` adapter tests through the matching
  `wasm-bindgen-test-runner` on Node 22/V8, in addition to cross-compiling the
  declared target matrix. This is real wasm runtime coverage, but does not
  replace the still-open Firefox/Safari browser golden gate.
- Rooted public `Value` and `Table` handles extend the existing retained
  `Function` model into a graph-preserving live embedding layer. They carry
  VM identity and decision epoch checks; expose binary strings, buffers,
  vectors, rich table keys, mutation, iteration, and raw equality; preserve
  shared/cyclic tables without leaking collector lifetimes; and cross both
  ordinary and typed invocations as live function arguments/results.
  `OwnedValue` remains the explicit acyclic snapshot layer, and snapshotting a
  cycle reports an error instead of silently losing identity. Creating a root
  is quota-checked and fails atomically when its registry node would exceed the
  VM logical-live memory limit; the low-level `Vm::retain_value` is therefore
  fallible, while `Vm::retain_values` admits multi-result batches atomically
  and keeps every candidate live across a pressure collection.
- `ServiceModuleResolver` and `ModuleSource` add host-owned script-visible
  `require` without an ambient filesystem dependency. Resolved canonical
  identities drive execute-once caches. Ordinary execution uses the
  decision-wide cohort; each installation uses a fresh lazy cohort selected by
  function origin and shared by its dependency graph. Nested referrers, cycle
  diagnostics, shared caller fuel, GC-aware cohort rooting, decision-reset clearing,
  deterministic-profile labels, panic poisoning, and logical-quota admission
  are covered and versioned by `RUSTIA_MODULE_RESOLUTION_ABI_VERSION`. Failed
  and multi-result modules are not cached; installation rollback includes
  required-module guest state. The CLI
  installs a filesystem-backed resolver in its standard and compatibility
  profiles with fixed `.luau`/`.lua`/directory-index candidate order,
  canonical cache identities, nested relative resolution, and module-source
  runtime diagnostics; deterministic CLI execution has no ambient filesystem
  loader.
- `RustiaExecutionProfile` provides canonical identity-component bytes and
  compact compatibility IDs for the selected core profile and guest surface,
  constructs the deterministic presets, and can be read back from both a
  builder and live VM. Embedders continue to bind settings, budgets, services,
  dependencies, and host ABI separately.
- `GuestSurface::HardenedLeaf` and
  `RustiaBuilder::deterministic_leaf()` add a fail-closed authoritative
  action-leaf environment layered on `DeterministicSim`. It requires an
  explicit logical-live memory limit, freezes globals/standard libraries/host
  namespaces, and removes or rejects environment/debug/raw mutation,
  output/clocks, VM RNG, buffers/vectors/coroutines, dynamic compilation,
  legacy extensions, global writes, and captured upvalues. The guest contract
  is versioned by `RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION`.
- `Rustia::compile_entry()`, `Invocation::call_entry()`, and
  `Rustia::invoke_entry_into()` execute a
  precompiled top-level decision body without installing an exports table,
  retaining a closure, or loading program metadata into the VM. Hardened reset
  preserves the sealed immutable configuration prefix while discarding every
  decision-local allocation; the lifecycle helper resets after both success
  and failure. The checked-in semantically validated ABBA benchmark measured
  the module control at 22.292–23.750 microseconds p95 and the hardened entry
  at 6.625–6.958 microseconds p95 in three matched local runs; this is
  workload-specific development evidence.
- `HostSchema` and `RustiaBuilder::build_for_host::<H>()` now separate a typed
  VM's lifetime-independent host API from the fresh `HostContext` borrowed by
  each `invoke`/`invoke_into`. Contexts may contain block-local input and slot
  borrows; callbacks receive only their
  declared `Input`, `Slots`, `Effects`, `SlotWrites`, or profile-gated
  `HostMut` view. Typed VMs execute guest modules and retained functions
  through a lexical `Invocation`, whose calls share one fuel budget and one
  bounded record-count/logical-byte journal. Records use a single dense
  sequence per invocation. Callback failure or panic rolls back that
  callback's suffix, while an error escaping the invocation rolls back all
  staged records; completed calls survive a later guest error caught by
  `pcall`. Transactional fuel exhaustion, OOM, record overflow, invalid
  boundary yields, a missing typed-host dispatcher, and host panic cannot be
  swallowed. Host panic poisons the VM; every later guest execution entry
  point rejects until `reset_decision()` succeeds. Containment includes typed
  and legacy callbacks, extension ops, stdout/clocks, and the `loadstring`
  compiler. Panic containment requires `panic = "unwind"`; aborting builds
  terminate or trap, so callbacks in those builds must not panic. Rollback is normally
  record-only: it does not undo guest heap mutation, `HostMut`, captured
  globals, or context interior mutation. `Invocation::install` deliberately adds a nested
  guest-state savepoint around its candidate; external host effects remain outside rollback.
  Successful `invoke` outcomes expose
  usage plus a borrowed reusable journal, while `invoke_into` pre-reserves and
  publishes into a caller lane without allocation during the final append and
  returns a non-borrowing usage receipt.
- `Vm::reset_decision()` / `Rustia::reset_decision()` implement decision-reset
  ABI v1. An idle reset reuses VM allocation containers while discarding guest
  globals, objects, upvalues, coroutines, loaded programs/caches, tracebacks,
  coverage, and retained roots; it restarts RNG, logical time, and deterministic
  GC scheduling. Settings, services, extension operations, installed host
  functions/modules, immutable compiled modules, and lifetime memory peaks
  persist. `Function`/`Exports` handles now carry a checked decision epoch, and
  a 600-decision mixed table/coroutine/program test proves an exact post-warmup
  VM-memory plateau. The shared mixed-heap pressure test adds a real Node/V8
  wasm lifecycle: 96 epochs combine rich object graphs, public roots, weak
  collection, hot reload, quota/OOM recovery, epoch invalidation, exact
  post-reset VM accounting, and stable second-half linear-memory pages.
- Runtime string atoms now use generation-checked reusable slots and ordinary
  tracing collections reclaim unreachable strings without allowing stale
  handles to alias replacements. `Vm::collect_garbage()` adds an explicit host
  quiescence boundary that completes reclamation, prunes unreachable coroutine
  state, and releases spare table, buffer, interner, and stack allocations to
  the process allocator while preserving retained values and compiled string
  constants.
- Loaded programs now use generation-checked reusable handles. Explicit host
  quiescence reclaims obsolete program bytecode, constants, inline caches, and
  coverage rows after the last reachable closure/coroutine/retained value drops;
  stale program handles cannot alias a replacement loaded into the reused slot.
- `Vm::memory_stats()` and the public facade expose VM-owned live/reserved
  allocation bytes plus opt-in instruction-boundary high-water marks. Heap
  marking uses compact per-store generation arrays rather than tree-set nodes,
  reducing temporary collection scratch while preserving the shared
  weak-table/reclamation graph.
- Logical-live memory is now maintained incrementally across heap stores,
  loaded programs, and VM control state. Quota admission and ordinary
  instruction boundaries no longer perform a whole-VM census; cold structural
  changes refresh only their affected aggregate before the next read. Exact
  reserved-byte telemetry remains on demand through `memory_stats()`, and
  opt-in peak tracking intentionally samples it at instruction boundaries.
  `collectgarbage("count")` now reports the maintained live total in KiB and
  `gcinfo()` reports its floored KiB value instead of compatibility counters.
  Exact-census test oracles cover every heap read, and exact-boundary table
  growth tests prove rejected array, short-string, and vector-key writes leave
  both the table and logical-live total unchanged.
- `VmSettings::memory_limit_bytes` and
  `RustiaBuilder::memory_limit_bytes(...)` cap VM-owned logical live bytes with
  identical native/wasm32 semantics. Limit pressure forces one collection,
  then returns catchable `OutOfMemory`; large string/table/buffer allocations
  are preflighted and growth writes stage a complete candidate before commit,
  so rejection is recoverable without a partial table mutation.
- The exact-commit `safe-memory-588b541` Sydney Linode cohort retained all 99
  native samples after confirmed server deletion and also passed the wasm32
  512 KiB quota/recovery probe. Against the frozen competitor cohort, Rustia
  now ties the 256 KiB best shared-workload peak, ranks first for allocation
  stress peak, second for stress retention, second for todo peak, and third for
  todo retention. Its shared-workload retained mean is 14.4 KiB versus
  mlua/Luau/JIT's 24.8 KiB.
- Executable conformance tracking is pending-free at 712 fixtures and 511
  behavior refs: 694/493 are Luau 0.709 oracle-backed and 18/18 are offline
  PUC 5.1.5 oracle-backed.
- VM strings and larger insertion-ordered table hash parts now use compact
  deterministic open-addressed `u32` side indices. The table index is built at
  16 hash entries, kept at or below 50% load, and releases its allocation after
  clear or contraction below the threshold.
- Loaded programs now store compact `u32` constant atoms and a 4-byte per-PC
  cache-site map. Eligible sites lazily allocate 16-byte atom/slot/shape records
  that always read the live table value and retain the first admitted atom to
  avoid polymorphic cache churn.
- The compiler now lowers non-import dot fields and compile-time string-key
  table reads, writes, assignments, and constructors through
  `GETTABLEKS`/`SETTABLEKS`.
  `GETTABLEKS` fuses the constant-string load and read at logical fuel 2.
  Compiler-emitted `SETTABLEKS` keeps its one-fuel key load before any
  intervening right-hand side and specializes the one-fuel final write; a
  no-preload fully fused bytecode mode also costs 2. Both retain live
  metamethod/readonly/mutation behavior. For compound updates, `GETTABLEKS`
  can also materialize the already charged key value for the later `SETINDEX`,
  preserving the unfused three-step load/read/write schedule.
- `NAMECALL` now fuses a constant method-name load with method lookup while
  leaving `CALL` as a separate operation. Its direct-receiver and live
  table-valued-`__index` cache lanes revalidate the current table, key, shape,
  metatable, and `__index` target and always read the live callee; no final
  callee, receiver, or class is cached. Lookup-only mode costs logical fuel 2,
  while the mode that also preserves the existing missing-method diagnostic
  costs 3. Each fused mode charges its full cost before side effects; the
  preloaded write mode preserves the original cross-RHS fuel/GC phase. Semantic
  fused-op costs remain deterministic, but aggregate GC checkpoints can move
  when specialization removes dead cleanup bytecode.
- Compiled constant storage is immutable, exposed through the read-only
  `Program::constants()` accessor, and shared across cloned programs and VMs;
  long compiled string atoms reference those shared bytes rather than copying
  them into every VM, including upgrading an equal runtime-owned atom when the
  compiled literal arrives later. Ordinary compiled literals and
  runtime-produced binary strings up to 13 bytes can live directly in the
  sealed 16-byte `Value`;
  constant-key/cache paths atomize them only when identity is required, and
  short-string table lookup misses, nil writes, and failed writes do not grow
  the VM atom table.
- `GETIMPORT` resolves validated one-to-three-component descriptors through
  live, shape/key-revalidated table slots. Compiler lowering respects lexical
  shadowing, host or script mutation remains observable, and path lengths one,
  two, and three cost logical fuel 1, 3, and 5 on both cache hits and misses.
- Compiler-predicted fixed-result `FASTCALL` sites cover unshadowed standard-library
  fields and binding-scoped aliases, guard the exact current function handle and
  native function kind, dispatch a matching native directly, and otherwise use
  the ordinary resumable call path. Reassignment, host replacement,
  metamethod calls, runtime arity errors, yields, and native variable-work fuel
  retain ordinary-call behavior; the base opcode costs one fuel on hit or miss.
- The public `rustia` facade has compile-checked examples for globals, typed
  host functions, deterministic extensions, retained script functions, serde,
  and hot reload.
- `rustia-cli` supports file, stdin, `--eval`, and `--repl` execution with
  profile, fuel, and source-name options.
- `rustia-cli` renders source-aware diagnostics for compile and runtime
  failures when rustia exposes source-span or runtime-line evidence.
- The benchmark harness has current-results JSON/HTML, generated per-workload
  claim gating, package-file history, the requested adapter matrix, and
  cross-adapter source coverage for the non-default `string_building` workload.
- Languages-by-target summaries rank implementations by their five-workload
  geometric mean, best first. The canonical report now presents implementation
  before language, uses ranked bars for the headline, workload heatmaps for
  detail, and a controlled-cohort ledger for history; one toggle switches both
  current-result views between absolute timing and speed relative to rustia.
  Recurring wasm publication now uses Node/V8 only;
  the accepted Deno, Bun/JavaScriptCore, and SpiderMonkey matrix remains archived
  as historical portability evidence.
- The dedicated Linux matrix v3 gate keeps zero-steal quiet prewindows, accepts
  timed groups at no more than 0.1% steal or one accounting tick, and retries a
  rejected group up to three times with output rollback and retained gate
  evidence inside the existing bounded Linode lifecycle.
- Benchmark calibration probes and timed repetitions now use freshly prepared
  engine/workload state with an untimed warmup, and `--samples-json` writes
  execution-ordered raw repetitions to a schema-identified sidecar without
  changing the append-only public history format.
- The benchmark harness now accepts `--fixed-n` for equal-work comparisons that
  bypass calibration while retaining fresh state and untimed warmup. Raw
  sidecars identify fixed versus calibrated selection; the documented protocol
  alternates baseline/candidate order and records three independent clean
  final-commit publication measurements. Fixed-N history appends now require a
  raw sidecar, written before each actual summary row, and accepted evidence is
  retained under the full candidate commit.
- A dependency-free fixed-sample analyzer validates sidecar identity and
  summaries, then emits deterministic pooled and hierarchical bootstrap JSON
  with a conservative-envelope verdict and explicit material-regression
  threshold; its self-test is part of benchmark preflight.
- A reproducible `just bench-do` workflow now owns the frozen DigitalOcean
  benchmark lifecycle: it validates the frozen Premium Intel VM catalog without
  silently changing plans, transfers the exact local Git objects, builds the
  baseline with the byte-identical candidate harness overlay, qualifies and
  CPU-pins a quiet Linux host, runs the predeclared eight-leg comparison, and
  publishes three actual final-binary rows only after the analyzer accepts the
  result. Per-run state supports exact-ID crash cleanup, and the Droplet plus
  ephemeral account SSH key are confirmed absent on every normal exit.
- Qualification-only `just bench-do-trials` profiles can compare regular and
  Premium Intel shared Droplets by build time, stability, conservative charge
  upper bound, and projected full-run charge; diagnostic mode cannot run or
  promote publication.
- `just bench-do-auth` now rotates its dedicated local `doctl` context before
  prompting, instead of silently reusing an expired or under-scoped token.
- A prospectively versioned `just bench-linode` v2 protocol now owns an ephemeral
  `g6-dedicated-2` instance after the original dedicated-KVM runs each observed
  one stolen scheduler tick. Old v1 blocks remain rejected; v2 retains zero
  steal in every pre-leg idle window and permits at most 0.1% during a timed
  leg, with the stability, contamination, ordering, analysis, and publication
  gates unchanged. It validates account backup defaults,
  account-specific region access, exact regional type/image metadata,
  whole-hour cost ceilings, and delete authority before billing; journals
  ambiguous creates by unique label; passes
  a local one-run SSH key directly at creation; and forbids evidence promotion
  until the instance is confirmed absent. Accepted exports whose local
  publication check rolls back can be retried offline by exact run ID, with
  archive, source, gate, raw-sample, recomputed-analysis, and history validation
  and without reading the token or launching another instance.
- The accepted Sydney Linode v2 comparison qualified at 0.50% pilot spread,
  kept peak timed steal to 0.074% with no contaminating processes, and measured
  2.42% lower candidate ns/unit than the baseline on pooled `dense_tables`
  samples. Its conservative nominal 95% interval was -3.35% to -1.57%, excluding
  the predeclared +5% material regression; three actual final-binary rows and
  the complete raw evidence are now retained in append-only history. Project
  status validation now accepts intentional partial controlled targets while
  still requiring unique result triples and exact target/engine/workload
  metadata coverage.
- A second exact Linode v2 comparison measured 65.959% lower candidate ns/unit
  on `oop_metamethods`, with pooled medians of 21,633.014 and 7,364.006
  ns/unit and a conservative nominal 95% interval of -66.39% to -65.55% against
  a zero-regression threshold. Three actual final-candidate rows and the full
  raw/gate evidence are retained. The comparison includes both the
  constant-field/method paths and a weak-table GC-scan correction, so the full
  delta is not attributed to one opcode family.
- The Linode coordinator now also supports an evidence-only exact 512-VM
  string-memory protocol. Sixteen accepted paired rounds produced 160 unique
  fresh-process samples; paired candidate-minus-baseline anonymous-memory
  contrasts saved 32.01 KiB/VM for unloaded short constants, 258.00 KiB/VM for
  unloaded long constants, 105.39 KiB/VM for loaded short strings, and 260.97
  KiB/VM for loaded long strings, leaving a 7.08 KiB/VM loaded-long residual.
  Its nine-file corpus is retained separately and cannot alter throughput
  history, results, or claims. Completed evidence-only exports can be promoted
  offline after confirmed deletion with their analysis recomputed.
- The VM hot loop now bypasses `gc-arena` when allocation debt is already zero
  and refreshes weak-table root snapshots only at possible collection or
  guest-entry boundaries. Focused tests preserve collection cadence and prove
  that an outer-only weak value remains live across nested guest execution with
  explicit `collectgarbage`.
- A second safe-Rust hot-loop pass adds guarded numeric arithmetic and array-slot
  lanes, exact `next`/`inext` iteration, scalar-math fast calls, compact inline
  register/return/upvalue storage, allocation-debt and weak-collection countdowns,
  and complete-bytecode-shape leaf-call plans for small counters and array
  iterators. Specialized paths retain ordinary fallback behavior for fuel,
  coverage, metamethods, coercion, mutation, recursion, and GC boundaries.
  Focused regression tests cover formatting, modulo, sorting, iterator cells,
  retained weak roots, and deterministic collection parity. A fresh local paired
  screen put rustia ahead of the Rust-native Ruau adapter on all five tracked
  workloads, but did not include the faster Luaur or `mlua-luau` adapters; the
  dedicated-Linux matrix below supersedes that directional result.
- The exact-commit `87db17542e19ae77ec92463f8a5b8e8b8a21a666`
  cross-engine Linode matrix accepted every native and wasm timed group on its
  first attempt with zero timed steal. Rustia median ns/unit improved
  34.97%/27.20%/27.44%/22.08%/22.77% on dense/sparse/closures/iteration/stdlib
  versus the prior committed Linux snapshot; the generated minimum-time
  `mlua-luau` geometric-mean gap fell from 10.1x to 7.30x. The VM was confirmed
  deleted before evidence and results promotion.
- The subsequent exact-commit `b1cd2841fb8fc2d916047391a1d757d53a1303cd`
  cross-engine Linode matrix accepted all native and wasm timed groups on
  their first attempt. Default numeric `table.sort` moved from 182,365.970 to
  166,135.066 median ns/unit (-8.90%) versus the immediately preceding Linux
  snapshot, and its minimum-time `mlua-luau` gap fell from 9.20x to 8.54x.
  The primary native group observed one steal tick (0.0118%, within the frozen
  0.1% allowance); every other timed group observed zero. The VM was confirmed
  deleted after 1,364 seconds and before evidence/results promotion, with a
  rounded compute-charge ceiling of $0.108.
- The exact-commit `fa8c8571d60659c4f637940acbb84eb591b1e039`
  cross-engine Linode matrix accepted all four timed groups on their first
  attempts. Against the Rust-native Ruau adapter, rustia won
  `generic_iteration` and trailed the other four workloads by 2.8%-35.0%, for a
  1.027x geometric-mean Ruau advantage. The optimized C++ Luau adapters remain
  the real target: Luaur and `mlua-luau` were 4.938x and 4.930x faster by
  five-workload geometric mean, with per-workload `mlua-luau` gaps from 3.265x
  to 6.667x. The VM was confirmed deleted after 2,227 seconds before the result
  was promoted offline with its complete gate and raw-sample evidence.
- An actual-bare-metal `just bench-vultr` fallback now follows rejected
  DigitalOcean shared and Linode dedicated-KVM placements without changing the
  frozen zero-steal protocol. It validates the Ubuntu 24.04 `vbm-4c-32gb` plan
  in Singapore, its one-hour `$0.164` minimum, and a two-hour `$0.328` safety
  ceiling for physical provisioning; supports a service user with the three
  least-privilege Vultr managed server policies; stores its
  private-prompt token mode `0600`, rejects virtualization, journals ambiguous
  creates, and confirms deletion before promoting evidence or the conditional
  three history rows.
- An actual-bare-metal `just bench-scaleway` fallback now targets the exact
  `EM-A610R-NVMe` Ryzen PRO 3600 Elastic Metal profile after two Vultr orders
  remained pending through their activation windows. It stores project-scoped
  credentials mode `0600`, owns a unique server and IAM SSH key per run, pins a
  separate cloud-init host key, rejects hardware or virtualization drift, and
  requires both resources to be confirmed absent before evidence promotion.
- A local fresh-state ABBA against `dac4ec2` found no remaining material
  regression above 5% after restoring the inline GC-debt hot path and separating
  ordinary calls from FASTCALL cache admission. Stdlib/string improved
  7.25%/19.71%; a cooled dense-table check retained a borderline 1.29% cost, so
  the result is recorded as hardening evidence rather than a blanket speed claim.
  Six uncontaminated fixed-work legs for the subsequent field/string working tree
  measured about +0.90% with overlapping spreads; the accepted exact-commit
  Linode comparison above now supersedes that working-tree diagnostic.
- Static bytecode and release-disassembly analysis showed that `dense_tables` does not execute
  `GETTABLEKS`, `SETTABLEKS`, or `NAMECALL`; its candidate lowering actually dispatches three fewer
  instructions per unit. A corrective hot-loop pass instead removes cross-cutting costs from the
  legacy numeric path: ordinary opcode fuel uses a base-cost lookup with validated special-cost
  fallbacks, numeric table access and ordering take number-first paths, and nil tests use the exact
  value tag. Logical fuel, invalid-program validation, numeric-key behavior, and string
  canonicalization remain unchanged. The accepted exact-commit Linode result now
  validates that corrective pass for this workload without supporting a blanket
  speed claim.
- The generated `bench/current-results.html` report has a release-preflight
  freshness check alongside its markdown, JSON, claim, and wasm inputs.
- Release preflight now composes release notes, package files, generated leaf
  package dry runs, progress HTML, project-status drift checks, benchmark
  result freshness, and workspace tests.
- Release preflight and hosted CI now enforce formatting plus strict Clippy on
  the pinned Rust toolchain.
- The separate fuzz workspace now receives pinned-toolchain formatting,
  strict Clippy, deterministic mutation sweeps, and smoke tests during release
  preflight.
- Hosted CI now cross-compiles the portable engine library for the declared
  macOS, Linux, Windows, iOS, Android, and browser-Wasm triples; real-host and
  browser correctness goldens remain a separate release gate.
- GitHub-hosted workflows pin `actions/checkout` 7.0.1 and
  `actions/setup-node` 7.0.0 by full commit SHA, removing the deprecated
  Node-20 action runtime while retaining explicit Node 22 project execution.
- The hosted release-preflight checkout now includes repository history needed
  by frozen benchmark-provenance self-tests.
- Crate publishability blockers are generated from cargo metadata and package
  file audits into `design/publishability.json`.
- GitHub Actions keeps benchmark work validation-only; fresh timing data comes
  from the explicit ephemeral Linode Linux lifecycle instead of an arbitrary
  self-hosted runner.
- The billed Linode lifecycle is now operator-started, accepts only an exact
  commit already present on `main`, and validates that ancestry before exposing
  credentials or running repository code.
- Workflow policy is checked in release preflight so hosted CI cannot record or
  publish fresh benchmark timings.
- Release preflight and hosted CI now compile the public `rustia` examples
  explicitly.
- The first public koto-to-rustia migration guide is documented in
  `design/76-koto-migration.md`.
- Crate metadata policy is generated from cargo metadata into
  `design/crate-metadata.json` and checked in release preflight.
- Typed host function conversion errors now include the host function name and
  argument or return position.
- The public `Rustia` facade has `eval<T>()` for typed conversion of a
  script result through `FromRustia`.
- `Rustia::load(...).source_name(...)` preserves host-provided source names in
  runtime diagnostics for every terminal operation.
- `rustia::Error` now exposes public source-span and runtime source-location
  accessors for host diagnostic renderers.
- Typed host functions can return `rustia::Result<T>` to report fallible host
  callback errors through the script runtime.
- Typed host functions and retained script calls now support up to eight
  typed arguments/results through the public tuple conversion traits.
- Owned-value conversions now cover `Option<T>`, `Vec<T>`,
  `BTreeMap<String, T>`, direct `OwnedTable` values, typed owned-table
  builders, typed string-key lookup, and nested table/array access for globals,
  host functions, and retained calls.
- Owned tables can deserialize string-keyed fields and array slots directly
  through serde.
- Retained script functions and export values are true collector roots checked
  by VM identity and decision epoch. They survive successful host-owned module
  replacement and fail closed after reset.
- The public `Rustia` facade now has capability-specific host function helpers,
  and `RustiaExtension` can mark external mutable-state reads explicitly.
- `RustiaExtension` exposes namespace plus per-function name, fuel,
  host-fuel ABI version, capability, parameter-count, parameter type, return
  type, and fallibility metadata before installation; the public metadata
  structs are non-exhaustive.
- Release-readiness docs now cover package metadata, package file-list audits,
  semver policy, changelog validation, and publish ordering.
- The VM now has a `VmServices` provider seam with shared stdout, wall-clock,
  monotonic-clock, and entropy sockets. `print` composes `tostring` per argument (honoring
  `__tostring`), joins arguments with tabs, appends a trailing newline, and writes to
  the attached sink, staying a no-op without one. No-argument `os.time` and
  default-timestamp `os.date` read the wall-clock provider and otherwise fall back to
  the pinned epoch 1_700_000_000; `os.clock` reads the monotonic provider. The `os`
  table is always registered on non-Sim profiles with all four members; on
  `DeterministicSim` it is registered iff at least one clock provider is plugged, and
  infallible VM constructors silently drop nondeterministic input providers. Empty
  entropy keeps the historical seed-zero `math.random` sequence; an entropy
  provider selects the initial seed once during VM construction.
- The public `rustia` facade gains environment presets: `RustiaBuilder::standard()`
  (ReproducibleStandard plus a real stdout print sink on native targets, and the new
  delegation target of `Rustia::builder()`), `deterministic()` (DeterministicSim, no
  ambient services), and `live()` (host stdout, system wall/monotonic clocks,
  system-seeded `math.random`, and `loadstring` enabled with the stock compiler).
  The `.stdout(...)`, `.wall_clock(...)`, `.monotonic_clock(...)`, and
  `.entropy(...)` sockets take a sink closure, `ServiceClock`, or
  `ServiceEntropy`; fixed/host-asserted deterministic providers and explicitly
  nondeterministic providers are distinct constructors. Native/WASI adapters are
  automatic. The default `web-live` feature adapts browser/JavaScript Wasm to
  `console`, `Date`, `performance`, and `crypto`; bare Wasm disables default
  features and plugs its inputs. `build()` returns typed errors for live inputs
  on `DeterministicSim` and for an incomplete requested live environment.
  `collectgarbage("count")`/`gcinfo` now read the maintained VM-owned logical-live total.
  Script-triggered and quota-forced collection also reclaims unreachable runtime-compiled
  programs while preserving active top-level and suspended-coroutine program roots.
- Provider-less `os.clock()` now reads a deterministic logical clock — executed
  instructions divided by 1_000_000 logical instructions per second — monotonically
  non-decreasing per VM and identical across peers and targets, so sandboxed scripts
  can benchmark themselves without a host provider.

### Changed

- The standalone wasm benchmark adapter now opts out of rustia's browser-live
  defaults, restoring the documented `--locked` release build without changing
  its pinned dependency set or pulling unused browser-service dependencies.
  The Node/V8 512 KiB quota/recovery probe remains covered; the maintained
  mixed-heap test now supplies the long-running reset and stable-linear-memory
  evidence.
- Default `table.sort` comparisons now handle numeric pairs directly in the
  native heap-sort loop, while string ordering, custom comparators, mutation
  checks, and dynamic comparison errors retain their existing paths.
- The VM's ordinary instruction loop now bypasses `gc-arena` when allocation
  debt is already zero and refreshes weak-table root snapshots only before an
  imminent collection boundary or a path that can enter guest code. Explicit
  collection, metamethod, fuel, and weak-root semantics remain covered by the
  conformance suite and a nested-call root-liveness regression test.
- The mutable public `Program::constants` field is now an immutable shared pool
  exposed through `Program::constants()`. This is an intentional `0.0.x`
  source-level API change that prevents compiled-string metadata from drifting
  after construction.
- Facade default `print` now writes to real stdout on native targets:
  `Rustia::builder()` delegates to `RustiaBuilder::standard()`, which installs a
  stdout sink (print stays a no-op on wasm32, and `Rustia::new`/`with_settings`
  remain service-free). Intentional `0.0.x` behavior change: scripts that
  previously printed into the void now emit visible output on native.
- Provider-less `os.clock()` now returns the deterministic logical clock instead of
  `0.0`. Intentional `0.0.x` behavior change: repeated calls observe monotonically
  non-decreasing values derived from executed instructions; a plugged monotonic
  provider overrides it on any profile, and `DeterministicSim` keeps no ambient
  clock unless a provider is plugged.
- `live()` now behaves like a conventional scripting runtime: it seeds the pinned
  PCG32 generator from host entropy and backs `os.clock()` with a true monotonic
  source. `standard()` and `deterministic()` retain the historical seed-zero
  sequence unless a replay-stable entropy provider is explicitly supplied.
- Rust-to-Luau conversion is now explicitly fallible: `IntoRustia`,
  `IntoRustiaMulti`, and the typed `OwnedTable` constructors return `Result`.
  Integer inputs and serde values/map keys are rejected when conversion to
  Luau's `f64` number type would round, while exactly representable wide powers
  remain accepted.
- `rustia-cli` now follows script-runner output semantics: guest `print` reaches
  stdout, ordinary chunk return values are ignored (including functions and
  cyclic tables), regular REPL statements are silent, and only `=expression`
  shorthand displays its value through Luau `tostring`.

### Fixed

- Suspended coroutine frames now participate in tracing only after their
  script-visible coroutine userdata is reachable. The collector follows
  coroutine-to-coroutine frame references to a fixed point, then reclaims
  unreachable suspended states and their cyclic object graphs; previously
  every suspended frame was an unconditional VM root until decision reset.
- `buffer.readf32`/`readf64` now preserve the exact IEEE-754 payload supplied
  by the buffer, including NaN sign/payload and negative zero, so immediate
  read/write round trips are byte-exact. f32 NaNs use an explicit cross-target
  widening/narrowing rule; arithmetic and ordinary numeric ingress continue
  to canonicalize engine-produced NaNs.
- Direct, tail, and stackless self-recursion now honor the configured VM
  recursion limit instead of the Luau conformance harness's 20,000-call
  compatibility ceiling. The pinned harness retains its oracle-specific frame
  accounting behind a conformance-only setting.
- Generic and generalized iterator calls can now yield through
  `coroutine.resume`; resumption applies the iterator results and continues at
  the correct loop edge.
- Interpolated-string lowering now remaps tokens, lexer errors, AST spans, and
  later runtime locations to the original source bytes, including nested
  interpolation.
- CLI file and stdin execution now accepts raw Luau bytes, preserves source
  offsets while masking a leading shebang, and resolves REPL runtime diagnostics
  against the chunk that defined the failing function.
- Every timed benchmark repetition is checked against the first checksum, and
  Linode promotion rejects any candidate ledger that changes or truncates the
  frozen published-history prefix.
- The `rustia-conformance` crate archive now contains the complete corpus,
  defaults to that bundled repository, and is package-verified by inventory,
  checksum, unpacked build, and full-manifest execution.
- Release preflight and hosted CI now invoke one canonical validation driver,
  Cargo metadata/build/test/package commands are locked, and the lockfile must
  begin clean. GitHub Actions are pinned to full commit SHAs.
- Every package archive now carries both license texts, and the package audit
  rejects README links whose relative targets are absent from the archive.
- The standalone browser demo disables host-import features, carries a locked
  dependency graph, and can be instantiated directly with an empty WebAssembly
  import object as documented.
- `os.time(table)` now defaults a missing `hour` field to 12 (noon) instead of 0,
  matching the pinned Luau 0.709 oracle (`VM/src/loslib.cpp` reads the field with
  `getfield(L, "hour", 12)`).
- `os.date("*t")`/`os.date("!*t")` date tables now carry a boolean `isdst` field
  set to `false`, matching the pinned oracle's `setboolfield(L, "isdst", tm_isdst)`;
  rustia is UTC-only, so `isdst` is always `false`. The non-`!` localtime
  divergence remains parked.

## Release Policy

- `0.0.x` is an unpublished/private-preview line.
- The first public preview will start at `0.1.0` only after the release gates in
  `design/90-release-readiness.md` are satisfied.
- Breaking public-facade changes after `0.1.0` require migration notes here.
- Semver and API stability rules are defined in `design/91-semver-policy.md`.
