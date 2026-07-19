# Changelog

All notable public-facing changes to rustia will be recorded here.

Current unpublished workspace version: `0.0.1`.

## [Unreleased]

### Added

- Luau 0.709 conformance tracking is pending-free, with the current manifest at
  692 fixtures and 491 behavior refs.
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
  preloaded write mode preserves the original cross-RHS fuel/GC phase, and all
  modes retain the equivalent instruction-count/GC cadence.
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
- A protocol-identical `just bench-linode` fallback now owns an ephemeral
  `g6-dedicated-2` instance after repeated DigitalOcean shared-CPU placements
  violated the zero-steal timed-leg gate. It validates account backup defaults,
  account-specific region access, exact regional type/image metadata,
  whole-hour cost ceilings, and delete authority before billing; journals
  ambiguous creates by unique label; passes
  a local one-run SSH key directly at creation; and forbids evidence promotion
  until the instance is confirmed absent.
- A local fresh-state ABBA against `dac4ec2` found no remaining material
  regression above 5% after restoring the inline GC-debt hot path and separating
  ordinary calls from FASTCALL cache admission. Stdlib/string improved
  7.25%/19.71%; a cooled dense-table check retained a borderline 1.29% cost, so
  the result is recorded as hardening evidence rather than a blanket speed claim.
  Six uncontaminated fixed-work legs for the subsequent field/string working tree
  measured about +0.90% with overlapping spreads; exact-final publication remains
  pending after two host-load-contaminated legs were excluded.
- Static bytecode and release-disassembly analysis showed that `dense_tables` does not execute
  `GETTABLEKS`, `SETTABLEKS`, or `NAMECALL`; its candidate lowering actually dispatches three fewer
  instructions per unit. A corrective hot-loop pass instead removes cross-cutting costs from the
  legacy numeric path: ordinary opcode fuel uses a base-cost lookup with validated special-cost
  fallbacks, numeric table access and ordering take number-first paths, and nil tests use the exact
  value tag. Logical fuel, invalid-program validation, GC cadence, numeric-key behavior, and string
  canonicalization remain unchanged. Controlled exact-commit measurement is still pending.
- The benchmark static site now has a release-preflight contract check for local
  assets, DOM bindings, JSONL history parsing, and current-results agreement.
- Release preflight now composes release notes, package files, generated leaf
  package dry runs, progress HTML, project-status drift checks, benchmark
  result freshness, and workspace tests.
- Release preflight and hosted CI now enforce formatting plus strict Clippy on
  the pinned Rust toolchain.
- Crate publishability blockers are generated from cargo metadata and package
  file audits into `design/publishability.json`.
- GitHub Actions now has hosted release-preflight plus self-hosted main-push,
  nightly, and manual benchmark workflows that publish to the private
  benchmark-data repository.
- Workflow policy is checked in release preflight so hosted CI and benchmark
  runner labels stay public and generic.
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

### Changed

- The mutable public `Program::constants` field is now an immutable shared pool
  exposed through `Program::constants()`. This is an intentional `0.0.x`
  source-level API change that prevents compiled-string metadata from drifting
  after construction.

## Release Policy

- `0.0.x` is an unpublished/private-preview line.
- The first public preview will start at `0.1.0` only after the release gates in
  `design/90-release-readiness.md` are satisfied.
- Breaking public-facade changes after `0.1.0` require migration notes here.
- Semver and API stability rules are defined in `design/91-semver-policy.md`.
