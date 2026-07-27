# rustia

A **deterministic** Luau interpreter written in Rust, designed for speed and low memory — built for a
deterministic game runtime and intended for public release as a standalone Luau 0.709-compatible Rust engine
once its conformance, benchmark, API, memory-lifecycle, and release gates are satisfied.

> **Status: P1 engine implementation is substantial but not release-complete.** The locked design has a broad
> production VM/compiler/API/conformance slice; bounded object/program reclamation, quotas/OOM, deterministic
> decision reset, and the typed per-invocation host transaction are implemented. The next release-critical work
> is O(1) accounting, broader mixed-heap pressure evidence, and the remaining module/value boundary.
> `spikes/` are throwaway prototypes that validated the design.
> Read **[`design/CURRENT.md`](design/CURRENT.md)** for the canonical state, then
> [`design/README.md`](design/README.md) for the full doc map.

## Highlights (spikes validated the direction; production gates are now in-tree)
- Performance claims are benchmark-gated per workload and target; no global speed headline is published until
  the committed claim gate approves it. A wasm-emission spike validates a hot monomorphic f64-kernel tier, but
  general Luau speed claims remain gated on type guards, fallback behavior, and benchmark history.
- `DeterministicSim` is bit-reproducible by construction on the tested matrix; `ReproducibleStandard` remains
  Luau-conformant while making reproducible choices for unspecified behavior.
- One portable engine: big-`match`→`br_table` interpreter plus a validated future wasm-emit JIT/AOT tier;
  **gc-arena-backed collection scheduling/storage scaffolding**, **16-byte tagged** values, a hand-written
  Luau parser, and thread-affine safe-Rust VM execution. Ordinary objects and loaded programs use
  reachability-based reclamation with generation-safe reuse.
- Interpreter performance foundation: compact deterministic open-addressed `u32` side indices for VM-local
  atoms and larger insertion-ordered hash parts; compact loaded-program constant atoms; a 4-byte per-PC cache
  site map with lazily allocated 16-byte shape/key/live-slot records; constant-key
  `GETTABLEKS`/`SETTABLEKS`; two-lane live receiver/`__index`-table `NAMECALL` lookup; and mutation-safe
  `GETIMPORT` plus guarded fixed-result `FASTCALL`, all with ordinary semantic fallbacks and logical fuel
  preserved. `NAMECALL` does not fuse the subsequent `CALL` or cache a final callee.
- Profiled hot-loop hygiene, reusable bounded private HotIR, direct immutable captures, and a four-lane
  prototype-polymorphic closure-call cache have exact dedicated-Linux evidence. In the latest `a562919` cohort,
  `closures_upvalues` median latency moved from 270.427 to 229.766 ns/call (15.036% lower); every new sample beat
  every prior sample. The generated five-workload minimum-time geometric-mean gaps are 3.494x to `mlua-luau`
  and 3.833x to Luaur. These are protocol- and workload-specific development results, not blanket speed claims.
- Compiled constant pools are immutable and shared across program clones and VMs. Long compiled string atoms
  reuse those shared bytes, while ordinary compiled literals and runtime-produced binary strings up to 13 bytes
  can be stored directly in the sealed 16-byte `Value`; constant-key/cache paths atomize only when needed, and
  short-key lookup misses and failed or nil writes do not grow the VM atom table.
- **No unsafe code in the public workspace:** all workspace crates, including the benchmark harness, inherit
  `unsafe_code = "forbid"`; research spikes and standalone wasm FFI demos are kept outside that product gate.
- Pinned to **Luau 0.709** (standalone luau.org surface, not Roblox).
- Current conformance runner: **694 fixtures**, **493 refs**, **0 pending-oracle**, **0 pending-impl**.
- Public embedding API first slice: typed `run<T>()`, owned values with
  `Option<T>`/`Vec<T>`/`BTreeMap<String, T>`/table conversions plus typed and nested table helpers, source-named
  run/compile helpers, typed/capability-tagged host functions with fallible `rustia::Result<T>` returns and up to
  eight arguments/results, retained script functions, pre-install host signature/fuel ABI metadata,
  retained-handle freshness checks and current-generation accessors, public source-span/runtime-location error
  accessors, owned-value/table serde, deterministic extensions, hot reload, and early macro sugar. `HostSchema`
  separates persistent function registration from a block-local `HostContext`; capability-specific views expose
  input, slots, ordered effects, slot writes, or profile-gated external mutation. Typed invocations share one
  fuel budget and stage a bounded count/byte record transaction. Ordinary object/program reclamation and the
  epoch-safe decision-reset ABI are implemented; module resolution, transactional installation/hot reload,
  rich graph values, and live zero-copy serde remain planned.
- Host services are host-pluggable through the facade: `RustiaBuilder::standard()`/`deterministic()`/`live()`
  environment presets, a `.stdout(...)` print sink with the real Luau composition (`tostring` honoring
  `__tostring`, tab-joined, trailing newline), and `.wall_clock(...)`/`.monotonic_clock(...)` sockets taking a
  determinism-labeled `ServiceClock` (`system()`/`fixed(t)`/`from_fn(f)`). Provider-less defaults stay
  deterministic: no-argument `os.time`/`os.date` read a pinned epoch, and `os.clock()` returns a logical clock
  (executed instructions / 1_000_000); `DeterministicSim` registers `os` only when a deterministic clock
  provider is plugged and rejects a nondeterministic one at `build()`.

## Embedding presets

The facade picks an environment in one word; `Rustia::builder()` delegates to `standard()`, and
`Rustia::new`/`with_settings` stay service-free. Finish with `build_for_host::<H>()` instead of `build()` when
constructing a typed `Rustia<H>`. Builder `.fuel(n)` is a VM-wide ceiling; a typed call uses the lower of that
ceiling and its explicit `InvocationLimits::new(fuel)` budget.

| | `deterministic()` | `standard()` (default) | `live()` |
|---|---|---|---|
| Profile | `DeterministicSim` | `ReproducibleStandard` | `ReproducibleStandard` |
| `print` | registered; no-op unless a host sink is attached | real stdout on native; no-op on wasm | real stdout on native; no-op on wasm |
| `os.time()` / `os.date()` | `os` absent unless a deterministic clock provider is plugged | pinned epoch 1_700_000_000 unless a wall-clock provider is plugged | system wall clock |
| `os.clock()` | follows the `os` registration rule | logical clock (executed instructions / 1_000_000) unless a monotonic provider is plugged | system monotonic clock |
| Pure `os` fns (`os.time(t)`, `os.date(f, t)`, `os.difftime`) | follow the `os` registration rule | always available | always available |
| `collectgarbage("count")` / `gcinfo` | not registered | registered; compatibility values pending O(1) incremental accounting | registered; same compatibility values |
| `loadstring` | opt-in host compiler only | opt-in host compiler only | enabled with the stock compiler |
| Host-fn capabilities | the five deterministic capabilities; external-state/nondeterministic registrations and nondeterministic clock providers rejected | all seven capabilities | all seven |
| GC | full collection on the deterministic instruction-count schedule | incremental/paced | incremental/paced |
| Reproducibility | bit-identical replay on the tested matrix | replay-stable defaults; real time only by explicit provider choice | not reproducible (live IO) |
| wasm32 | supported | supported (same script-visible values; print no-op) | `build()` fails loudly — plug providers instead |

`math.random` stays pinned in every profile. Clock providers carry a determinism label
(`ServiceClock::system()` is nondeterministic; `fixed(t)`/`from_fn(f)` are host-asserted deterministic), and
the facade `build()` rejects a nondeterministic clock on `deterministic()`.

## Project Pillars
The canonical decision filter is [`design/02-pillars.md`](design/02-pillars.md). The short version:

- **Pinned Standalone Luau Compatibility** — Luau 0.709 standalone runtime behavior, not Roblox/latest-Luau drift.
- **Conformance and Provenance** — claims need fixtures, upstream links, audits, benchmarks, or explicit defers.
- **Deterministic Profiles and Replay ABI** — reproducible public behavior plus a stricter lockstep profile.
- **Rollback-Safe Host Boundary** — hosts own authoritative state, rollback capture, and ordered effect commits.
- **Layered Rust Embedding API** — small typed facade first; advanced interop stays layered.
- **One Safe Native + Wasm Engine** — one semantic engine across native and `wasm32-unknown-unknown`.
- **Benchmark-Gated Performance Claims** — no blanket speed headline without generated claim approval.
- **Public-Clean OSS Consumer Contract** — generic crates, docs, semver, CI, packaging, and no grabby-specific API.

## Project Progress
The detailed progress view is generated from [`design/progress.json`](design/progress.json) into
[`design/progress.html`](design/progress.html). These are rough engineering estimates as of 2026-07-28 for the
public rustia project; actual grabby integration work is explicitly out of scope.

| goal | complete | confidence |
|---|---:|---:|
| Whole public rustia project | 86% | 84% |
| Usable embedded scripting crate | 92% | 92% |
| Full public standalone Luau engine release | 58% | 83% |
| Standalone benchmark OSS project | 82% | 88% |
| Public release readiness | 78% | 88% |

- Current conformance runner: **694 fixtures**, **493 refs**, **0 pending**.
- The benchmark harness has all 12 requested adapters, an explicit verified `mlua-luau-jit` mode, and a
  non-default `string_building` workload source set across them; Lua 5.4 runs separately from Luau because
  `mlua-sys` cannot link both in one binary.
- Current benchmark claim posture is generated at [`bench/CLAIMS.md`](bench/CLAIMS.md); it approves no blanket
  speed headline from the current snapshot.
- The committed benchmark protocol keeps raw samples, fixed-work evidence, process/steal gates, exact source and
  toolchain identity, and fail-closed promotion after VM deletion. The current clean `83bc2f3` Linux snapshot has
  65 rows across 12 adapter families plus the separate mlua/Luau JIT mode and five default workloads. The
  interpreter-only minimum-time gaps are 3.483x to `mlua-luau` and 3.833x to Luaur; verified JIT mode is 4.283x
  rustia overall and 8.036x on closures, with its compile/first-call costs reported separately. Earlier
  dense/OOP/string/HotIR cohorts remain traceable historical evidence. Every result is specific to its workload,
  target, and protocol; details are in [`design/56-performance-research.md`](design/56-performance-research.md).
- Exact-commit cohort `safe-memory-588b541` now supplies the first dedicated-Linode native pressure evidence
  for reclamation/telemetry/quota work and passes the wasm32 512 KiB quota/recovery probe. Rustia ties the best
  shared peak at 256 KiB and retains 14.4 KiB across the shared set versus mlua/Luau/JIT's 24.8 KiB.
  The same Node/V8 probe now runs 300 decision resets and verifies an exact post-warmup VM-owned-memory plateau.
- Major remaining gates, in order, are an O(1) quota-accounting fast path and script counters; broader
  mixed-heap/native/Wasm pressure coverage; module loading, rich values, and transactional installation,
  compiler/runtime semantic-tail
  hardening, the full cross-target correctness/golden matrix, broader benchmark history, and release packaging.
  Semver policy is tracked in [`design/91-semver-policy.md`](design/91-semver-policy.md),
  release notes in [`CHANGELOG.md`](CHANGELOG.md), koto migration guidance in
  [`design/76-koto-migration.md`](design/76-koto-migration.md), crate metadata audit in
  [`design/crate-metadata.json`](design/crate-metadata.json), and package file-list review in
  [`design/package-files.json`](design/package-files.json). Current publish blockers are generated at
  [`design/publishability.json`](design/publishability.json).
- `just release-preflight` validates formatting, strict Clippy, release notes, package files, generated leaf
  package dry-runs, progress/project status, benchmark results, crate metadata, workflow policy, examples, and
  workspace tests without running fresh benchmark measurements; `.github/workflows/ci.yml` mirrors that gate on
  hosted Linux CI using the pinned Rust toolchain.

## Layout
- `design/` — the design record: constraints → research → spikes → architecture (ADR) → conformance → improvements.
- `conformance/` — executable Luau fixtures and the upstream Luau 0.709 test-port plan.
- `crates/` — production P1 workspace: VM, lexer, parser, compiler, public API, CLI, and conformance runner.
- `spikes/` — throwaway prototypes (skeleton VM, libm cross-arch, wasm-JIT, gc-arena, …) with their measurements.

## Examples
- `cargo run -p rustia --example embedding` — globals, fallible typed host functions, deterministic extensions, retained script functions, and serde.
- `cargo run -p rustia --example hot_reload` — compile/install hot reload plus retained-handle survival and freshness checks.
- `cargo run -p rustia --example typed_invocation` — a block-local host context, capability views, shared fuel,
  bounded ordered records, and allocation-safe publication into a caller-owned lane.

`Rustia<H>` stores only a lifetime-independent `HostSchema` and its registered callbacks. Each `invoke` or
`invoke_into` borrows a fresh `HostContext<H>`, so tick-local input and slot slices need not be `'static`.
Typed VMs run guest code only through the lexical `Invocation`; several `Function::call_in` operations share
the invocation's fuel and record budget. Successful effects and slot writes receive one dense sequence across
both kinds, starting at zero for each invocation.

Record staging is transactional: a failing or panicking callback loses the records appended by that callback,
and an error escaping the outer invocation publishes no records. A guest `pcall` that catches a later guest
error does not erase already completed host calls. Fuel exhaustion, OOM, record count/byte overflow, an invalid
yield boundary, a missing typed-host dispatcher, and host panic are hard transaction aborts. The transactional
OOM/yield rule is scoped to typed `Invocation` execution and does not redefine ordinary legacy execution. A
host panic poisons the VM: every later guest execution entry point rejects until `reset_decision()` succeeds.
Containment covers registered host functions, typed dispatch, legacy extension ops, stdout and clock services,
and the configured `loadstring` compiler.
Rollback covers Rustia's private record journal only—not guest heap
mutations, `HostMut`, callback captures, or context state changed through interior mutability.

For grabby, make at most one invocation successful for each `(tick, phase, emitter)` and append with
`invoke_into`; this makes the per-emitter sequence unique before lanes are stably sorted by `(emitter, seq)`.
The returned `InvocationReceipt` reports fuel, record count, and logical record bytes without borrowing
Rustia's journal.

Long-lived hosts can call `Vm::collect_garbage()` at an idle boundary to finish
reclamation and release spare VM-owned allocations to the process allocator.
Values that must survive that boundary need an active `RetainedValue` lease
from `Vm::retain_value`.

Deterministic per-decision hosts can call `Rustia::reset_decision()` at an idle
boundary. Reset ABI v1 keeps VM settings, services, extensions, and installed
host functions/modules, but discards all guest state, restarts RNG/logical
time/GC scheduling, and invalidates every prior `Function`/`Exports` handle.
Host-owned `CompiledModule` values remain reusable; ordinary inserted globals
must be rebound for the next decision. Replay protocols should record
`RUSTIA_DECISION_RESET_ABI_VERSION`.

## CLI
- `cargo run -p rustia-cli -- path/to/script.luau`
- `cargo run -p rustia-cli -- --profile deterministic-sim --fuel 100000 --eval "return 40 + 2"`
- `cargo run -p rustia-cli -- --source-name stdin.luau -`
- `cargo run -p rustia-cli -- --repl`

Compile and runtime failures include source-name-aware line snippets and caret markers where rustia has source
span or runtime-line evidence.

Built design-first with Claude Code.

## License
Licensed under either of **[Apache License, Version 2.0](LICENSE-APACHE)** or **[MIT license](LICENSE-MIT)**
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional
terms or conditions.
