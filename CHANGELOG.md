# Changelog

All notable public-facing changes to rustia will be recorded here.

Current unpublished workspace version: `0.0.1`.

## [Unreleased]

### Added

- Luau 0.709 conformance tracking is pending-free, with the current manifest at
  694 fixtures and 493 behavior refs.
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
- Crate publishability blockers are generated from cargo metadata and package
  file audits into `design/publishability.json`.
- GitHub Actions keeps benchmark work validation-only; fresh timing data comes
  from the explicit ephemeral Linode Linux lifecycle instead of an arbitrary
  self-hosted runner.
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
- The public `Rustia` facade now has `run<T>()` for typed conversion of a
  script result through `FromRustia`.
- `Rustia::run_named<T>()`, `run_owned_named()`, and named compile/run helpers
  preserve host-provided source names in runtime diagnostics.
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
- Retained script functions and export tables now expose the successful install
  generation they came from, making hot-reload handle semantics explicit.
- Retained function/export handles now expose freshness checks, and
  `Function::call_current` rejects stale handles after a successful hot reload.
- Export tables now have current-generation `value`, `deserialize`, `get`, and
  `function` accessors for fail-closed hot-reload integrations.
- The public `Rustia` facade now has capability-specific host function helpers,
  and `RustiaExtension` can mark external mutable-state reads explicitly.
- `RustiaExtension` exposes namespace plus per-function name, fuel,
  host-fuel ABI version, capability, parameter-count, parameter type, return
  type, and fallibility metadata before installation; the public metadata
  structs are non-exhaustive.
- Release-readiness docs now cover package metadata, package file-list audits,
  semver policy, changelog validation, and publish ordering.
- The VM now has a `VmServices` provider seam with shared stdout, wall-clock, and
  monotonic-clock sockets. `print` composes `tostring` per argument (honoring
  `__tostring`), joins arguments with tabs, appends a trailing newline, and writes to
  the attached sink, staying a no-op without one. No-argument `os.time` and
  default-timestamp `os.date` read the wall-clock provider and otherwise fall back to
  the pinned epoch 1_700_000_000; `os.clock` reads the monotonic provider. The `os`
  table is always registered on non-Sim profiles with all four members; on
  `DeterministicSim` it is registered iff at least one clock provider is plugged, and
  infallible VM constructors silently drop a nondeterministic clock provider.
- The public `rustia` facade gains environment presets: `RustiaBuilder::standard()`
  (ReproducibleStandard plus a real stdout print sink on native targets, and the new
  delegation target of `Rustia::builder()`), `deterministic()` (DeterministicSim, no
  ambient services), and `live()` (real stdout, system clocks, and `loadstring`
  enabled with the stock compiler). The `.stdout(...)`, `.wall_clock(...)`, and
  `.monotonic_clock(...)` sockets take a sink closure or a `ServiceClock`
  (`system()` nondeterministic, `fixed(t)` deterministic, `from_fn(f)` host-asserted),
  and `build()` returns typed errors for a nondeterministic clock provider on
  `DeterministicSim` and for `live()` on wasm32. `collectgarbage("count")`/`gcinfo`
  stay canned until honest memory accounting lands, and `math.random` stays pinned in
  every profile.
- Provider-less `os.clock()` now reads a deterministic logical clock — executed
  instructions divided by 1_000_000 logical instructions per second — monotonically
  non-decreasing per VM and identical across peers and targets, so sandboxed scripts
  can benchmark themselves without a host provider.

### Changed

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

### Fixed

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
