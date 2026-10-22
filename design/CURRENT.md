# rustia — CURRENT canonical state (read this first)

> **This file is the single source of truth for "where the project actually is."** Other design docs are
> staged snapshots and may lag; where they disagree with this file, **this file wins.** (Created 2026-06-30 to
> fix version-drift flagged by 3rd-party review finding H1; updated 2026-10-19.)

## One-paragraph state

The follow-up compatibility, benchmark and application API audit is recorded in
[`94-release-compatibility-audit.md`](94-release-compatibility-audit.md).
The compatibility fixes merged in PR #70 as `9b12a5a8` after required CI passed.
Eight-leg remote acceptance resolved the dense-table regression: +0.04%, with
its 99% interval spanning zero. The fresh 13-engine comparison measured that
merge and landed in PR #71; the README now cites its immutable cohort.
Application review retained the accepted API and corrected WOTS's ignored
per-call fuel limit through the existing invocation API; closeout PR #72 merged.
The next diagnostic follow-up fixes all 13 recorded differences against the
pinned Luau oracle (27 original cases now match), with expanded validation-order
and alias coverage. Final CI, review and remote screening remain delivery gates.
The numeric-index follow-up specifies checked signed-64 conversion and corrects
the rounded upper-bound bug; native regression coverage passes and cross-target
CI remains a delivery gate. See [95](95-numeric-index-contract.md).
Real Firefox correctness passed in both feature modes; Safari and additional
native execution remain open. The prepared platform workflows and evidence are
recorded in [96](96-browser-platform-acceptance.md).
The broader public-release audit remains open; this is not full Luau parity or
authorization to publish packages.

**Current milestone:** private 0.x API and real-application readiness, tracked in
[`92-first-release.md`](92-first-release.md). The immutable acceptance candidate is
`92a85d65eb33724c17103d43477d34a633392768`; its API inventory, tests, application
pins, CI and merge gates are recorded in
[`93-private-api-acceptance.md`](93-private-api-acceptance.md). Keep publication
disabled and the repository private. The workspace remains at `0.0.1`; this
milestone does not authorize a public candidate or 1.0 promotion. The future 1.0
promise will cover the embedding API and CLI, with lower-level engine packages
separately versioned at 0.x.

The README starts with executable examples and plain-language feature/speed
comparisons. Rustdoc starts with a working example and a short task index, with
conversion, ownership, reset, limits and failure rules next to the relevant
methods. Facade-owned settings/errors and VM/epoch-safe handles keep application
code independent of interpreter implementation types. Required checks and
independent reviews gate every rollout PR; the private baseline does not close
the broader public-release checklist.

**The application API boundary merged in [PR #63](https://github.com/slowchop/rustia/pull/63)**
as `8f8558abf361e939599d2c093614ae0776682d26`. The facade owns settings,
profiles/capabilities, memory samples, source/compile/runtime errors, and
source diagnostics. Builder configuration replaces raw VM constructors/settings;
`Default` matches the standard preset. Callback adapter traits are sealed marker
bounds, while owned conversions and borrowed typed contexts remain application
extension points. Settings have opaque versioned identity bytes covering all
effective options. A captured non-string guest error returned through Rust is
re-raised as its original text, avoiding stale/foreign raw object IDs; protected
calls entirely within guest code keep object identity. Required CI and independent
reviews passed for that boundary. WOTS [#7](https://github.com/slowchop/wots/pull/7)
merged as `97b87889`, and civfun [#51](https://github.com/civfun/civfun/pull/51)
merged as `6aaa3466`, using exact reviewed Rustia revision `030cb038`.
The final common-candidate rollout is WOTS [#8](https://github.com/slowchop/wots/pull/8),
civfun [#53](https://github.com/civfun/civfun/pull/53), and Grabby
[#1083](https://github.com/slowchop/grabby-next/pull/1083). The earlier facade
pins above remain historical evidence; final pins and gates are in the acceptance
record. The CLI accepts `-V`/`--version`.
See [the boundary contract](75-embedded-api.md#application-api-boundary).

**Focused builtin error-phase fixes merged in [PR #64](https://github.com/slowchop/rustia/pull/64)**
as `71524dba99b277622f27e0522c8740f611c37547`.
Unknown standard-library fields and argument lists outside a specialized opcode's
shape use ordinary calls, so runtime failures no longer reject valid source or
unreachable branches during compilation. The oracle-verified fixed-arity
builtins ignore trailing arguments after evaluating them; `table.insert` still
rejects four arguments. `pairs`/`ipairs` calls with extra arguments use the normal
iterator path. Trailing calls and varargs retain their actual result counts, including zero;
parentheses select one value. Recognized direct builtin callees resolve after
argument execution, with explicit import metadata preserving that order across
repeated coroutine suspension. Variable-length argument chains preserve expansion
beyond the fixed instruction shapes, and recursive calls resume those chains
through explicit VM frames across functions, programs and callable receivers.
Ordinary bytecode fallbacks share those frames after resumption.
Pending protected-call values stay rooted during fallible program retention.
Aliases and parenthesized callees keep ordinary
lookup order. Four-profile failure/recovery and call-shape regressions, eight
new pinned-Luau fixtures, and expanded bounded differential cases cover this
slice. The batch passed required CI, independent review and remote screening
before merge. The integrated module-loading/resource follow-up is tracked in the
acceptance record; the broader language-parity audit remains open.

**Yieldable `require` is implemented in the current private follow-up.** A
coroutine-owned loading continuation keeps pending module execution alive and
publishes the zero/one-result cache only on completion. Same-owner cycles and
another coroutine's in-progress identity report errors without taking over the
load. Ordinary module failure keeps completed guest writes and typed records
when caught; no whole-VM savepoint spans unrelated coroutine execution. A failed
outer installation still restores guest/cache/coroutine state and provisional
records. Close, abandoned-coroutine collection, hard abort, and reset release
loading ownership. Module-resolution ABI v3 records this changed failure and
suspension contract. Ordinary callable receivers now use the same VM continuation driver, preserving
yield/resume and bounded recursion without growing the native stack. The pinned
Luau oracle requires `__call` itself to be a function; callable table metamethods
are rejected in both ordinary and expanded calls. Native/Node tests cover suspended loaders and typed-object
aliases; the bounded stateful driver also interleaves load/resume/close/reset and
installation rollback. Repeated-string output work charges the same variable fuel
before allocation in specialized and ordinary calls; four-profile overlap/recovery
tests cover both limits. Source, independent reviews and final CI gates are linked
from the acceptance record.

The [private delivery order](92-first-release.md#delivery-order) groups the work
into a green baseline, facade boundary, construction/failure contracts, typed
Rust objects, runtime gaps, Linux/macOS-arm64/Node-V8 correctness, and an exact
candidate used by WOTS/civfun/Grabby. Runtime and platform work can run alongside
API work. The full native/mobile/Firefox/Safari runtime matrix remains required
before public release, but does not block this private milestone.
The [private platform checks](84-private-platform-acceptance.md) now add one
shared exact transcript on Linux x86-64, native macOS arm64, and Node/V8 in
default and bare modes. Native jobs pass explicit targets; the Mac job also
verifies the actual OS, architecture and compiler host. A clean external Cargo
application reuses public contract tests
against the five extracted runtime archives with only facade/Serde dependencies
and candidate-lock package identities. Native Mac and Node jobs have executed
the integrated candidate; the complete job logs and Linux/final acceptance gates
are linked from the acceptance record.
PR #60's stale generated benchmark-progress report was repaired in
[PR #61](https://github.com/slowchop/rustia/pull/61), merged after full preflight,
platform/Node-V8, differential smoke, and remote screening checks passed. The
repair changed no measurement or performance claim.

**Typed Rust objects merged in [PR #65](https://github.com/slowchop/rustia/pull/65)**
as `2187c8d16ec5c222bb01ed15ab68950569e937ce`.
`ObjectType<T, H>` registers methods and `Object<T>` keeps a VM/epoch-bound live
instance. Callbacks accept object handles and return existing or fresh objects
through `NewObject<T>`. Deterministic methods read replay-stable input and stage
effects/slot writes in the existing journal. Standard/live methods may change
payloads or external state immediately, outside guest rollback. Complete
registrations reject forbidden capabilities atomically; wrong/stale/foreign
receivers fail before argument conversion. Definitions survive reset, while
instances are released even if stale handles remain. Replacement changes future
instances. VM metadata/roots are quota-accounted; arbitrary Rust payload
allocations remain host memory. Public and private tests cover these contracts;
civfun's Player integration merged in [#52](https://github.com/civfun/civfun/pull/52)
as `2787ff8ae0afa913fc6a8f9a667427fef93d7443`. Its final common pin, 62 host tests
and saved-map acceptance are recorded alongside the other clients. See
[the object guide](83-rust-objects.md). Generic mutable-payload snapshots remain deferred.

**Memory-limit hardening:** memory-limited VMs now avoid batched execution paths
and use the scalar dispatcher's instruction-boundary quota checks, closing the
former HotIR/tape collection-window gap. Cross-profile, cross-interval regressions
and instrumentation-free execution counters cover this policy. See the
[quota-window resolution](58-vm-core2.md). Screening evidence is linked from the
acceptance record; no local timings or new speed claim are used.

[Error behaviour and fuzzing](82-error-behaviour-fuzzing.md) is complete in
[PR #34](https://github.com/slowchop/rustia/pull/34): cross-profile failure/recovery
regressions, bounded stateful embedding and isolated pinned-Luau differential
fuzzing, and the quota-limited PUC short-string placement fix are merged.
Both six-target remote campaigns pass, including expanded stateful coverage at
`347b0af`. Canonical release preflight, final platform/Node-V8 CI and remote PR
benchmark screening pass. WOTS #6, civfun #44 and Grabby #1027 are merged after
their full rollout gates and required reviews, using exact upstream revision
`aee98e7815460c2df014c9b20f92de8de5382814`.
This is not a release-completion or performance claim.

[Human-friendly script diagnostics](79-script-diagnostics.md) merged in
[PR #32](https://github.com/slowchop/rustia/pull/32), with host-only source
snapshots, expression spans, typed errors, bounded reports, and console source
mapping. Canonical release preflight, full correctness/conformance tests,
platform/Node-V8 CI and remote PR screening pass. Controlled-client changes are
recorded in WOTS #5, civfun #43 / CIV-75, and Grabby #1025. This completes neither
the wider public-release gate nor the remaining language error-phase parity work.

The **design phase is complete and reviewed**, and P1 implementation is now substantial in production crates.
The current workspace contains the VM/lexer/parser/compiler/public API/CLI/conformance runner, plus an
executable Luau 0.709 conformance corpus with **0 pending-oracle** and **0 pending-impl** rows. The public
release gate also formats, strictly lints, and smoke-tests the separate fuzz workspace. Its stable deterministic
baseline mutates 2,048 frontend cases and 2,048 VM byte streams; the VM decoder covers all 131 production
opcodes exactly once under bounded input, fuel, recursion, and a 512 KiB logical-live quota. Hosted CI executes
the bare and default web-live wasm adapters on Node 22/V8 as real WebAssembly runtime tests in addition to
cross-compiling the declared target matrix; Firefox/Safari browser goldens remain open. The public
embedding facade from [`75-embedded-api.md`](75-embedded-api.md) is implemented at the first useful level:
owned values with typed and nested table helpers, a composable `load(...).source_name(...)` execution surface,
typed host functions with fallible `rustia::Result<T>` returns, pre-install host signature/fuel ABI metadata,
reclaimable `create_function` values and shared `HostFunction` definitions for persistent registrations/modules
with mixed owned/live arguments and tuple results,
VM/decision-epoch-safe retained script function handles, public source-span/runtime-location error
accessors, owned-value/table serde with symmetric externally tagged enum support (including Serde rename
rules), snapshot-free deserialization from rooted live values, atomic extension registration, host-owned module
replacement, macro sugar, and `eval`/`eval_live`/`eval_serde`/`exec`/`install` terminals over borrowed
text/byte source, diagnostic names, and rooted custom lexical environments. `install` creates a fresh
globals-inheriting environment by default, uses a fresh lazy `require` cohort, and publishes rooted exports only
after the complete candidate succeeds. The
typed host transaction is now concrete: `HostSchema` keeps registration lifetime-independent while every
`invoke`/`invoke_into` borrows a fresh, possibly block-local `HostContext`; capability-derived views expose
input, slots, effects, slot writes, or profile-gated external mutation. Typed VMs execute guest code only through
a lexical `Invocation`, whose calls share one fuel budget and bounded record-count/logical-byte journal.
Decision-reset ABI v1 is now implemented at both VM and facade layers. An idle reset reuses the VM's allocation
containers, preserves profile/settings/services/extensions/installed host functions and modules plus
host-owned immutable compiled modules and lifetime peaks, and discards every guest global, object, upvalue,
coroutine, loaded program/cache, traceback, coverage row, and retained root. RNG, logical time, compatibility
GC state, and deterministic collection scheduling restart from construction values; legacy execution fuel is
fresh per call and typed invocation fuel is explicitly shared. Every retained `Function`/`Exports` clone is
epoch-checked and becomes stale after reset. Focused tests
cover active-execution rejection and an exact post-warmup memory plateau across 600 mixed
table/coroutine/program/retained-export decisions. A shared native/Node-V8 mixed-heap test now runs 96
decision epochs and 3,072 waves under a 2 MiB logical-live quota. It combines bounded public roots, cyclic
tables, strings, closures, suspended coroutines, userdata, buffers, vectors, weak tables, module replacement, atomic
OOM recovery, and epoch invalidation; both targets reach the same exact post-reset VM plateau, and warmed wasm
linear-memory pages remain constant.
The generic authoritative-leaf layer is now implemented
([`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md)):
`RustiaBuilder::deterministic_leaf()` layers a frozen, allowlisted
`GuestSurface::HardenedLeaf` on `DeterministicSim`, requires an explicit logical-live memory limit, and excludes
environment/debug/raw mutation, output/clocks, VM RNG, buffers/vectors/coroutines, dynamic compilation, and
captured upvalues. `RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF` is now the single typed source for
constructing that preset, its canonical profile/surface identity-component bytes, and its compact compatibility
ID; both the builder and live VM expose the selected descriptor so embedders can attest those core semantics
instead of mirroring them in a label. Settings, budgets, services, dependencies, and host ABI remain separate
embedder-owned identity inputs. `Rustia::compile_entry` plus `Rustia::invoke_entry_into` execute a host-owned top-level
decision body without exports-table, retained-closure, or loaded-program setup and reset after success or
failure. Hardened reset preserves only the checked immutable pre-guest heap prefix and discards all
decision-local allocations; sealing rejects both prefix holes and pre-seal external roots. The checked-in
complete-lifecycle benchmark validates the compared workloads. Performance evidence must come from
`ci-bench`; local development timings are not accepted evidence.
The host services bus from [`72-host-services.md`](72-host-services.md) is now implemented: facade presets
`standard()`/`deterministic()`/`live()`, a host stdout sink giving `print` its real Luau composition,
determinism-labeled wall/monotonic clock providers feeding `os.time`/`os.date`/`os.clock`, and a deterministic
logical `os.clock` fallback (executed instructions / 1_000_000) replacing the former `0.0`. The same bus
includes determinism-labeled entropy for the initial `math.random` seed: `standard()` and `deterministic()` keep
seed zero unless a replay-stable provider is explicit, while `live()` takes system entropy, real clocks, stdout,
and `loadstring`. Native/WASI adapters are automatic; browser `wasm32-unknown-unknown` uses the default
`web-live` JavaScript adapter (`console`, `Date`, `performance`, `crypto`), and bare Wasm can disable it and
inject the three input providers.
The typed record journal assigns one dense sequence across effects and slot writes. A failed/panicking handler
rolls back its own suffix; an error escaping the invocation publishes nothing; a completed host call remains
complete if a later guest error is caught by `pcall`. Transactional fuel exhaustion, OOM, count/byte overflow,
invalid boundary yield, a missing typed-host dispatcher, and host panic are hard aborts; the transactional
OOM/yield rule does not change ordinary legacy execution. A caught host panic poisons the VM, so every later
guest execution entry point rejects until decision reset. Containment includes host functions, typed dispatch,
legacy extension ops, stdout/clocks, and the `loadstring` compiler. Panic containment requires an unwinding
build; aborting builds terminate or trap, so their callbacks must not panic. General invocation rollback covers
staged records only—not guest heap writes, `HostMut`, captured callback state, or context interior mutation.
Installation adds a nested guest-state savepoint that restores heap/global/coroutine/RNG/logical-time/GC/module
cache state and provisional roots on failure; external host and resolver effects remain outside it. `invoke`
exposes the reusable journal plus fuel/count/byte usage; `invoke_into` pre-reserves and
publishes to a caller lane with no allocation at final append. Callback captures remain a semantic trust
boundary. Grabby must allow only one successful invoke per `(tick, phase, emitter)`, because the dense sequence
restarts at every invoke.
A from-scratch Rust Luau
interpreter is validated as **GO** by the earlier spikes and is now backed by the production conformance suite;
performance is continuously benchmark-gated, and the generated gate does not yet approve a blanket public speed
headline. All 8 architecture axes are decided
([`30-architecture.md`](30-architecture.md) v3), 10 conformance-verified improvements are adopted
([`55-improvements.md`](55-improvements.md)), the no-unsafe SotA performance research stack is now captured in
[`56-performance-research.md`](56-performance-research.md), and the standalone benchmark harness has the engine/workload/history
shell, generated current-results HTML report, 19 workload entries including cross-adapter `string_building` source coverage and
six synthetic/realistic memory workloads, and
the `rustia`/`boa`/`koto`/`luaur`/`mlua-lua54`/`mlua-luau`/`rhai`/`roto`/`ruau`/`rquickjs`/`wasmi`/`wasmtime` native adapters in
place.
The benchmark artifacts now also include a first-class wasm pre-lock gate source
(`../bench/wasm-gate.json`), recurring wasm target results for `rustia`, `koto`, and `rhai` under Node/V8, and
retained historical V8, JavaScriptCore-family, and SpiderMonkey portability evidence. Generated report panels
make the required production rustia wasm measurements explicit.
Its generated [`../bench/CLAIMS.md`](../bench/CLAIMS.md) gate keeps public benchmark language per-workload and
currently approves no blanket speed headline from the committed snapshot.
The public API now has release-preflight-enforced compile-checked examples for embedding and hot reload, plus a
first koto-to-rustia migration guide.
The public workspace now treats **no unsafe code** as a product feature: every workspace package inherits
`unsafe_code = "forbid"`, and the live VM is thread-affine instead of relying on an unsafe cross-thread `Send`
marker. Experimental spikes and standalone wasm FFI demos remain outside that public workspace gate.
The hardened table/atom/inline-cache performance foundation is now in-tree: VM-local byte atoms and larger
insertion-ordered hash parts use compact deterministic open-addressed `u32` side indices; loaded programs use
compact `u32` constant atoms plus a 4-byte per-PC site map; and eligible sites lazily allocate 16-byte
atom/slot/shape records that always read the live value. Mutation-safe `GETIMPORT` resolves one-to-three-component
descriptors without caching final values, and generic fixed-result `FASTCALL` guards the exact native function
handle before direct dispatch and otherwise follows the ordinary resumable call path. Non-import dot fields and
compile-time string-key reads/writes now lower to `GETTABLEKS`/`SETTABLEKS` with the same live slot
revalidation as generic index operations. `GETTABLEKS` costs fuel 2. Compiler writes keep a one-fuel key load
before an intervening RHS and then use the preloaded-key, one-fuel `SETTABLEKS`; the no-preload fully fused
bytecode mode costs fuel 2. Compound updates retain the key produced
by `GETTABLEKS` for their later `SETINDEX`, preserving the unfused three-step load/read/write fuel schedule.
`NAMECALL` fuses the constant-name load
and method lookup, not the subsequent `CALL`: its two lanes cover the receiver and the receiver's current
table-valued `__index`, revalidate both paths, and read the live callee without caching a final value, receiver, or
class. Lookup-only `NAMECALL` costs fuel 2; the mode that also preserves the prior missing-method check costs 3.
Fused costs are charged before their side effects and preloaded writes retain the original cross-RHS phase.
Their semantic instruction costs remain deterministic, but aggregate GC checkpoints can move when compiler
specialization removes dead cleanup bytecode.
Insertion order, metamethod/readonly behavior, mutation visibility, and logical fuel remain preserved.
Compiled constant pools are immutable and shared across program clones retained by different VMs. Long compiled
string atoms reference that storage without copying the bytes per VM, and an equal runtime-owned atom upgrades to
the shared bytes when the compiled literal arrives later. Ordinary loaded literals and runtime-created binary
strings of up to 13 bytes can live directly in the sealed 16-byte `Value`. Constant-key
and cache paths still atomize when identity is required, and table insertion canonicalizes a new non-nil short
key, but lookup misses, nil writes, readonly writes, and invalid-table writes do not create atoms.
The first profiled hot-loop hygiene pass now avoids entering `gc-arena` when allocation debt is already zero and
refreshes weak-table root snapshots only before a possible collection boundary or an instruction that can enter
guest code. Focused tests cover ordinary collection cadence plus nested guest execution with explicit
`collectgarbage`, while the full pending-free conformance suite preserves the public semantics.
`rustia-cli` supports file, stdin, `--eval`, and `--repl` execution with profile, fuel, source-name options,
state-preserving line evaluation, expression shorthand, and source-aware diagnostics for compile and runtime
failures.
CI wiring now includes pinned-toolchain formatting and strict Clippy gates, hosted release-preflight checks,
and explicit public-example compilation. Hosted workflows validate committed benchmark artifacts but never take
fresh timings; those come only from the attested owned ci-bench host.
The operational rollout is specified in
[`81-benchmark-operations.md`](81-benchmark-operations.md): shared, normally
CPU-busy self-hosted runners remain valid for correctness and artifact work but
are not timing authorities; the generated benchmark report now exposes
machine-readable progress freshness plus explicit landscape pause and active
artifact-only publication state instead of appearing silently current.
The dashboard also has a separately typed Grabby application-build section with
reviewed sequential ci-bench comparisons, debugger/runtime tradeoffs and selected
public evidence. It does not enter Rustia engine rankings or histories. Memory
promotion now delegates dashboard publication to the same current-main publisher,
so historical measurements cannot replace the page with an old renderer. See
the September 9 section of `81-benchmark-operations.md`; live publication remains
subject to final PR checks and deployment verification.
The benchmark harness prepares fresh engine/workload state for calibration and every recorded repetition, warms
that state once outside the timer, and can write execution-ordered raw samples to a separate
`rustia.bench.raw-samples.v1` JSON sidecar. `--fixed-n` bypasses calibration and uses exactly the selected work for
every warmup and timed repetition; sidecars record whether `n` was fixed or calibrated, while the stable public
history remains min/median summary JSONL. The equal-work protocol alternates ABBA/BAAB order and requires at least
three independent publication measurements of the clean final commit, with the sidecar written before each actual
summary append. The accepted exact-commit Linode v2 comparison measured 2.42% lower corrective-candidate ns/unit
on pooled `dense_tables` samples, with a conservative nominal 95% interval of -3.35% to -1.57%, excluding the
predeclared +5% material regression. Qualification spread was 0.50%, every contaminant gate passed, maximum timed
steal was 0.074% under the 0.1% ceiling, and three actual final-binary rows plus raw evidence are now in history.
Earlier diagnostic legs reduced the working-tree dense difference to about +0.90% with overlapping spreads; two later
legs were excluded after unrelated release builds saturated the host. The preceding local fresh-state ABBA
against `dac4ec2` found no material regression above 5% after hardening; stdlib/string improved materially,
closures were neutral-to-faster, and the cooled dense-table confirmation retained a borderline 1.29% slowdown.
Static bytecode and release-disassembly analysis showed that `dense_tables` executes none of the new
`GETTABLEKS`/`SETTABLEKS`/`NAMECALL` opcodes and dispatches 6,964 candidate instructions per unit versus 6,967
for the baseline. A corrective pass therefore targets cross-cutting legacy-path tolls: ordinary fuel
classification uses a base-cost lookup with validated special-cost fallbacks, numeric table access and ordering
take number-first paths, and nil tests use the exact value tag. Logical fuel, validation, numeric-key semantics,
and string canonicalization remain unchanged. The accepted result closes the controlled dense
material-regression gate for this workload and Linux protocol; it is not a blanket release speed claim.
The same dedicated Linode protocol compared `ae3beae2025feb21d0e37c70168204ae154ae125` with exact candidate
`f208c12d0ce266117743569d736346fae9a35db2` on `oop_metamethods`: candidate ns/unit was 65.9594% lower, with a
conservative nominal 95% interval of -66.3876% to -65.5482%, excluding the predeclared zero-regression threshold.
Three final candidate rows and the complete raw evidence are in history. This comparison covers the complete
candidate, including both the constant-field/method paths and its weak-table GC-scan correction; it does not
attribute the full delta to one opcode family.
An evidence-only 512-VM sampler then compared that baseline with exact candidate
`1370d6ccefba7a1956441247c3139af2a2cf31fa` over 16 accepted paired rounds and 160 fresh processes. The landed
shared/inline string representation removed 32 KiB/VM of unloaded short-constant growth, 258 KiB/VM of unloaded
long-constant growth, and 105.39 KiB/VM of loaded-short growth relative to baseline; loaded-long growth fell by
260.97 KiB/VM and retained a 7.08 KiB/VM residual. Descriptive medians show a 4 KiB larger one-time short-program
image and slower program construction in the sampled scenarios, while combined VM creation plus program
execution was faster for every scenario; those timing summaries do not carry the paired confidence intervals of
the primary memory contrasts. This is exact string-focused Linux evidence, not a complete mixed-heap RSS budget.
The subsequent exact-commit `87db17542e19ae77ec92463f8a5b8e8b8a21a666` cross-engine Linode matrix accepted
all native and wasm timed groups on their first attempt with zero timed steal. Against the preceding committed
Linux snapshot, rustia median ns/unit improved 34.97% on `dense_tables`, 27.20% on `sparse_tables`, 27.44% on
`closures_upvalues`, 22.08% on `generic_iteration`, and 22.77% on `stdlib_math_table`. The generated minimum-time
claim gate's `mlua-luau` gap therefore moved from a 10.1x geometric mean (7.1x-14.1x range) to 7.30x
(5.25x-9.53x). This is still development evidence rather than a blanket release claim. The matrix's ephemeral
VM was confirmed deleted before its evidence, native rows, and Node wasm rows were promoted.
The following exact-commit `b1cd2841fb8fc2d916047391a1d757d53a1303cd` matrix accepted all timed groups on
their first attempt and measured the focused numeric default-`table.sort` guard. Against the immediately prior
snapshot, rustia dense-table minimum/median ns/unit improved 7.42%/8.90%, and its minimum-time `mlua-luau` gap
fell from 9.20x to 8.54x. One primary-native steal tick was 0.0118%, within the frozen 0.1% allowance; all other
timed groups observed zero. The VM was confirmed deleted after 1,364 seconds and before promotion, with a
$0.108 rounded compute ceiling. Independent-host movement elsewhere left that snapshot's five-workload gap at a
7.29x geometric mean over a 5.09x-10.02x range.
The exact-commit `d4ec275e19bad0efcf8e96db4030e7380bd93a9b` matrix gates the replacement of the
recursive bespoke numeric-loop kernel with the reusable private `HotIr`. The IR is bounded and source-ordered,
uses reused unboxed numeric scratch, stages every fallible operation before its single permitted table effect,
and deopts to the already-fetched interpreter instruction without replay. Independent review found no semantic
or safety defects; explicit Off/Force parity tests cover all numeric operators, policy gates, deterministic GC,
atomic guard failures, and partial deopt. Against the immediately preceding `3732b80` Linux snapshot, native
median changes were -0.23% dense, -7.14% sparse, +5.74% closures, +5.12% generic iteration, and -0.59% stdlib,
for a +0.47% normalized geometric mean. The exact Linux result therefore preserves aggregate performance but
does not reproduce the local -29.31% dense screen; that local result remains directional only. At that snapshot,
the minimum-time gap was 3.495x to `mlua-luau` (2.688x-4.650x range) and 3.757x to Luaur
(2.722x-5.154x), while rustia beats Ruau on all five workloads. All timed groups passed on their first attempt;
one native steal tick was 0.0095%, all other groups observed zero. The VM was confirmed deleted after 1,529
seconds and before promotion, with a $0.108 rounded compute ceiling.
The prior exact-commit `a5629199643847c0da78792644ad3656887bee21` matrix gates prototype-polymorphic
closure-call fast paths plus direct immutable-upvalue captures. The compiler preserves mutable loop/backedge
capture semantics by discovering captured bindings before final lowering; the VM uses a four-lane call-site PIC
keyed by program/prototype and falls back for policy, guard, recursion, fuel, coverage, and GC boundaries. In the
committed dedicated-Linux matrix comparison, rustia `closures_upvalues` median time moved from 270.427 to
229.766 ns/call (15.036% lower latency, 1.177x throughput); every new raw sample beat every prior sample. Those
Linux snapshots used separate but identically specified dedicated instances, so the cross-host delta is
corroborating rather than a paired-host attribution.
The generated current five-workload minimum-time gaps are 3.494x to `mlua-luau` and 3.833x to Luaur; rustia
beats Ruau on four of five by minimum time. The same cross-host snapshot comparison moved rustia median latency by +2.35% on
`dense_tables`, +12.70% on `sparse_tables`, +8.93% on `generic_iteration`, and +5.74% on
`stdlib_math_table` (five-workload normalized geometric mean +2.45%). Because these are separate instances,
they are follow-up signals rather than attributed regressions; sparse and generic iteration should receive the
next stability check. All timed groups passed on their first attempt, and the VM was confirmed deleted
before the evidence and generated results were promoted, with a $0.108 rounded compute ceiling.
The current exact-commit `83bc2f370ee4149b6fa6e11eca951b41b4cccbd2` matrix adds an explicit
`mlua-luau-jit` mode without changing the interpreter-only headline. A matched JIT-off/JIT-on numeric sentinel
produced the same checksum and a 3.843x native-execution signal before any JIT row was admitted. Across the five
common workloads the reference interpreter is 3.483x rustia by geometric mean and the JIT mode is 4.283x;
closures improve from 64.67 to 28.52 ns/call under JIT, while dense tables remain essentially flat. Preparation,
first-call, combined cold-start, and warmed steady-state samples are all retained; the JIT mode's geometric-mean
cold cost is 677.26 microseconds versus 385.10 microseconds for the reference interpreter. All four timed groups
passed on their first attempt, the primary group observed one steal tick (0.00867%), and the VM was confirmed
deleted after 1,558 seconds before the 65 native rows, 15 wasm rows, and evidence were promoted.
The current exact-commit `34f6deff4a42501b6d66d257249eecfcbca1976a` matrix was measured with the cached
table length-border slice present. That slice's targeted dense_tables Linode A/B measured +1.7569% candidate
ns/unit (conservative interval +0.02% to +3.63%, regression excluded but no win), and its sparse_tables A/B
measured +0.6158% (interval -4.25% to +2.03%, neutral), so the bet was falsified and the cache was reverted
at `74340c4`; only its spare-capacity mutation-version correctness fix survives. Versus the preceding
`83bc2f3` snapshot — 86 commits earlier, so per-workload movements are stability signals, not single-change
attribution — rustia median ns/unit moved -7.07% dense, +8.06% sparse, +20.97% closures, +16.10% generic
iteration, and +17.32% stdlib. The generated current five-workload minimum-time gaps are 3.944x to
`mlua-luau` and 4.336x to Luaur, and rustia beats Ruau on three of five workloads by minimum time. All four
timed groups were accepted on their first timed attempt with zero timed steal, and the VM was confirmed
deleted after 1,560 seconds before the 65 native rows, 15 wasm rows, and evidence were promoted, with a
$0.108 rounded compute ceiling.
Two further exact-gated bets were falsified since: a GETIMPORT+FASTCALL fusion opcode was reverted
uncommitted after local screening showed a ~0.02% tracked-suite ceiling and a ~3% loss on a
dispatch-dominated synthetic, and a per-call-site generalized-iterator leaf-step cache was reverted at
`bbe0c5c` after its accepted generic_iteration Linode A/B measured +3.3591% candidate ns/unit
(conservative interval +1.58% to +4.87%). Both are recorded as falsified-bet evidence in
[`56-performance-research.md`](56-performance-research.md), not as speed claims.
A follow-up profile-driven slice promotes the pure register-bookkeeping opcodes `Move` and
`ClearLocalCell` (42-61% of executed instructions on four tracked workloads) into the common-instruction
fast path; its accepted controlled-host A/B measured -8.4508% candidate median ns/unit on
`generic_iteration` (conservative interval -9.064% to -7.348%; its rented-host evidence was removed on
2026-09-13 and remains in Git history), with public-history publication
deferred to the next full-matrix cohort under the dedicated-cohort policy.
`tools/check-workflows.js` keeps those workflow policies inside `just release-preflight`.

## Project pillars
The canonical decision filter is [`02-pillars.md`](02-pillars.md). These pillars define what counts as current
rustia work and what should be deferred or marked out of scope:

| # | pillar | current meaning |
|---:|---|---|
| 1 | Pinned Standalone Luau Compatibility | Luau 0.709 standalone runtime behavior: parser/compiler/VM/source-visible stdlib, vector-3, flags-off. |
| 2 | Conformance and Provenance | Compatibility claims need fixtures, upstream links, generated audits, benchmark records, or explicit defer reasons. |
| 3 | Selectable Environment Contracts and Replay ABI | `standard()` has reproducible defaults with explicit opt-in nondeterminism, `deterministic()` is the strict lockstep/replay subset, and `live()` intentionally makes no replay guarantee. |
| 4 | Rollback-Safe Host Boundary | The host owns cross-tick authoritative state, typed slots, rollback capture, ordered effect queues, and commits. |
| 5 | Layered Rust Embedding API | Small typed facade first; extensions, capabilities, fuel, retained handles, serde, and future interop are layered. |
| 6 | One Safe Native + Wasm Engine | One semantic engine across native and `wasm32-unknown-unknown`, with no public unsafe-code or target-specific semantic fork. |
| 7 | Benchmark-Gated Performance Claims | Public speed claims are per workload/target and generated from committed benchmark history. |
| 8 | Public-Clean OSS Consumer Contract | Public crates stay generic, documented, packageable, semver-scoped, and free of grabby-specific API. |

Important boundary: "Luau compatibility" means pinned standalone Luau runtime compatibility. It does not make
Roblox APIs, latest-Luau churn, Lua 5.4 behavior, analyzer/typechecker parity, bytecode loading, or raw Luau C
API parity release-critical work.

## Known functionality gaps (honest release boundary)

The pending-free conformance corpus proves the named 517-behavior denominator: 499 Luau 0.709 refs plus 18
offline PUC 5.1.5 compatibility refs. It does **not** mean that every production concern or host-facing
facility is finished. The highest-impact gaps are:

1. **Bounded object lifetime and memory policy.** Ordinary reachability now reclaims and generation-safely
   reuses tables, functions, upvalues, atomized runtime strings, userdata, buffers, and vectors. Public retained
   values are collector roots. At an explicit idle boundary, `Vm::collect_garbage()` completes two reclamation
   passes, removes coroutine state whose userdata is unreachable, compacts bounded table/buffer/interner
   storage, and shrinks VM/coroutine/traceback stack capacity. Loaded programs also use generation-safe slots:
   obsolete program bytecode, constants, inline caches, and coverage rows are reclaimed only after no reachable
   closure, coroutine, or retained host value refers to them. Compiled constant atoms stay rooted while their
   program is live, and stale object/program handles cannot alias replacements. Script-triggered and
   quota-forced collection now applies the same program cleanup while preserving active top-level and suspended
   coroutine program roots. `Vm::memory_stats()` provides a
   VM-owned live/reserved census and opt-in instruction-boundary peaks. `VmSettings::memory_limit_bytes` now
   enforces that logical-live boundary with one forced-collection retry, catchable OOM, preflight for large
   string/table/buffer requests, and candidate-before-commit growing table writes. Heap stores and loaded
   programs maintain live totals; VM control state uses a cached aggregate with local refresh after structural
   changes. Quota admission and ordinary instruction boundaries therefore avoid a whole-VM census, while exact
   reserved telemetry remains on demand. Script-visible `gcinfo`/`collectgarbage("count")` read that same
   maintained logical-live total. Public retained-root registry nodes are also admitted against the quota and
   fail atomically rather than letting live-handle creation cross the boundary. Exact-commit Linode cohort
   `safe-memory-588b541` retained 99
   native samples after confirmed deletion, passed the wasm32 512 KiB quota/recovery probe, tied the 256 KiB
   best shared peak, and reduced the shared retained mean to 14.4 KiB versus mlua/Luau/JIT's 24.8 KiB. The
   maintained native/Node-V8 mixed-heap pressure gate now adds 96 epochs, 3,072 allocation waves, exact reset
   plateaus, weak/cyclic reclamation, module replacement, public roots, and atomic oversized-allocation recovery.
   External allocator/page-return and process-RSS history remain measurement work, not missing VM
   reclamation semantics. GC interval tuning follows this landed evidence.
2. **Host-provided runtime services and modules.** The output/clock/entropy provider slice is implemented
   ([`72-host-services.md`](72-host-services.md)): `print` composes `tostring` per argument (honoring
   `__tostring`), tab-joins, and writes to a host stdout sink when one is attached; no-argument
   `os.time`/`os.date` read a wall-clock provider with a pinned-epoch fallback; and `os.clock` reads a
   monotonic-clock provider with a deterministic logical-clock fallback. Entropy selects the initial
   `math.random` seed, and native/WASI/browser adapters supply live providers. The typed per-invocation context,
   capability views, shared fuel, bounded ordered record journal, rollback/`pcall` contract, usage receipts,
   panic poisoning/reset, and atomic typed `HostModule` registration are implemented. Script-visible `require`
   is now backed only when the host installs a `ServiceModuleResolver`: resolution returns a canonical identity
   and source. Ordinary execution has one decision-wide cache; every installation receives a fresh lazy cohort
   shared by its dependency graph and selected by function origin. Reachable cache results remain roots, nested requests receive the
   referrer's source name, cycles report their identity path, reset clears the caches, and module execution shares
   caller fuel. `DeterministicSim` rejects resolvers not explicitly labeled replay-stable, hardened leaves reject
   dynamic modules, the behavior is versioned by `RUSTIA_MODULE_RESOLUTION_ABI_VERSION`, and the core VM never
   reads an ambient filesystem. The separate CLI adapter supplies fixed-order `.luau`/`.lua`/directory-index
   lookup, canonical cache identities, nested relative resolution, and module-source runtime diagnostics in
   standard and compatibility profiles; deterministic CLI execution leaves `require` absent.
   Host-facing VM memory telemetry and script-visible counters now read the maintained VM-owned logical-live
   total without a whole-VM census at ordinary instruction boundaries. Extension batches and module candidates
   now install atomically. Each installation owns a fresh lazy dependency-cache cohort selected by function
   origin; unreachable cohorts are collector-pruned and old rooted functions remain callable. Required modules
   now use coroutine-owned continuations, retain loading ownership across suspension, and publish the cache
   only on completion. Close, collection, reset and failure release ownership; the ABI v3 contract and
   final candidate evidence are in [the private acceptance record](93-private-api-acceptance.md).
3. **Complete public value interop.** The advanced live-handle spine is implemented: VM/epoch-safe rooted
   `Value`/`Table`/`Function` handles preserve binary strings, buffers, vectors, identity-bearing keys,
   shared/cyclic table graphs, mutation, and raw equality without exposing collector lifetimes. Live values
   can cross ordinary and typed invocations, and `OwnedValue` remains the deliberately small acyclic snapshot
   layer. Live `globals()`, quota-aware `create_table()`, and owned-key/live-value insertion are implemented;
   the composable `load(source).source_name(...).environment(...)` chunk builder is also implemented,
   with environment-handle validation before compilation and exact environment capture by nested functions.
   Hardened leaves reject mutable constructor authorities before allocation and dynamic loading before
   compilation. Snapshot-free serde deserialization now reads rooted `Value`, `Table`, and `Exports` graphs
   directly into owned Rust types while keeping the eager `OwnedValue` snapshot explicit. Typed Rust-object
   registration, live handles, and methods merged in PR #65 under
   [83-rust-objects.md](83-rust-objects.md), with real civfun Player use merged in #52.
   The exact common candidate and consumer validation are recorded in
   [93-private-api-acceptance.md](93-private-api-acceptance.md).
   Synchronous function construction now shares typed owned/live conversions, capability/fuel metadata,
   and contextual dispatch with persistent registrations and host modules. Heap functions own callbacks;
   guest aliases survive replacement, collection releases unreachable captures, and reset discards
   disposable ownership while restoring only the newest persistent configuration. A public
   `Thread` handle remains intentionally absent until a real host-side scheduler caller defines its contract.
   Full mlua-style `UserData`/`Scope`, async callbacks, and any mlua migration shim remain later optional
   layers. These APIs serve every builder preset: `deterministic()` enforces replay restrictions, `standard()`
   permits explicit nondeterminism over reproducible defaults, and `live()` is a first-class intentionally
   non-reproducible environment rather than a restricted or second-class mode.
4. **Semantic-tail and diagnostics hardening.** Continue differential parser/compiler testing, keep valid
   runtime errors catchable at runtime rather than prematurely rejecting them at compile time, and maintain
   the implemented bounded stack/source diagnostics. Module/extension installation failure is now transactional; external
   host callback and resolver effects remain deliberately outside rollback. The final audit closed untracked
   `buffer.readf32`/`readf64` NaN-payload loss by
   separating exact buffer-bit ingress from canonicalized arithmetic results and giving f32 NaNs an explicit
   cross-target widening/narrowing rule. New gaps require focused Luau-0.709 oracle fixtures.

### Deep-audit reconciliation (2026-07-29)

The multi-agent review and final repository-wide sweep found no remaining `TODO`, `FIXME`, `todo!`, or
`unimplemented!` marker in production crates, fuzz targets, release tools, or hosted workflows. It did find
and reconcile three kinds of documentation drift: the implemented memory benchmark was still marked pending
in the index; resolved conformance decisions (deterministic reference IDs, three-wide vectors, buffer bit
operations, OOM, recursion, and the absent `load` global) were still labeled open; and a later `__le` row
contradicted the source-verified Lua-5.1 fallback recorded earlier in the same inventory. The exact remaining
release boundary is the four numbered gaps above plus the full target/golden and packaging gates in
[`90-release-readiness.md`](90-release-readiness.md); historical watch-list rows are evidence prompts, not
silent claims of completion.

## 🚀 Next actions (cold-start handoff — do in this order)
**Read order for a fresh agent:** `AGENTS.md` (auto-loaded) → **this file** → [`02-pillars.md`](02-pillars.md)
→ [`30-architecture.md`](30-architecture.md) (the ADR)
→ the doc for your task. (A fresh Claude session in this repo also auto-loads the project memory.)

**Action queue:**
The mixed-heap action is closed: one maintained source now executes 96
native/Node-V8 epochs and 3,072 waves across cyclic graphs, module replacement, retained
handles, weak tables, quotas/OOM, exact reset plateaus, and stable warmed wasm
linear-memory pages. It also exposed and closed unconditional rooting of
unreachable suspended-coroutine frames.

1. **Finish the host-facing runtime boundary and ordered advanced API track.** Follow
   [`75-embedded-api.md`](75-embedded-api.md#build-order): the composable chunk builder is now implemented on
   the graph-preserving live `Value`/`Table`/`Function` spine and its live globals/table constructors. Build
   on the implemented synchronous host-function API and verified controlled-client integrations by finishing
   stack diagnostics. Transactional installation and host-owned
   replacement are implemented. The worker-local/per-call host
   context is already implemented. Keep the existing typed facade as the simple layer, keep the advanced layer
   available to all builder presets, and do not leak grabby-specific types, internal GC lifetimes, or a raw C
   API. `live()` remains the explicit no-reproducibility-guarantee environment; deterministic policy is
   enforced only by constructors that promise it.
2. **Harden the production engine** against the LOCKED decisions ([`30-architecture.md`](30-architecture.md) §1–§4):
   keep the 16-byte tagged `Value` firewall, portable `br_table`-friendly dispatch, instruction-count fuel,
   deterministic f64/libm behavior, table-key `-0` normalization, profile-specific GC behavior, and the public
   workspace's `unsafe_code = "forbid"` gate. The stable bounded fuzz baseline now mutates the frontend and all
   131 VM opcodes; continue coverage-guided and differential work for newly discovered compiler and
   runtime-error-phase edges, and close the full native/browser correctness-build-golden matrix without adding
   developer-machine timings to benchmark history.
3. **Continue measured performance work** using [`56-performance-research.md`](56-performance-research.md).
   The dense-table, constant field/namecall, shared-string, HotIR, and closure-call slices now have accepted
   Linux evidence; the cached length-boundary slice was exactly gated, falsified (no win on dense or
   sparse), and reverted. Region Engine v1 is implemented as private effect-certified consecutive-instruction
   metadata with exact deoptimization and deterministic-policy fallback. Its accepted controlled DigitalOcean
   A/B measured +1.3181% candidate ns/unit with a conservative 95% interval of -4.5763% to +3.9306%, excluding
   a material 5% regression without establishing a speedup. Treat the slice as performance-neutral within the
   measured interval. The next programme is now [`58-vm-core2.md`](58-vm-core2.md). Its C2.1 cleanup/register
   mini-tier was exactly falsified on DigitalOcean: candidate `2bb7bef` was 5.293% slower on fixed
   `generic_iteration`, with a conservative nominal 95% interval of +3.720% to +6.135%. The coordinator rejected
   publication, public benchmark history stayed unchanged, and cloud cleanup was confirmed. C2.2 removes the
   competing per-`Move` tier and implements direct register receipts inside one uninterrupted Region, resolved
   once per function. The receipt slice passed focused semantic review, complete correctness validation, and its
   controlled DigitalOcean promotion gate: exact candidate `84e732a` used 8.1651% less time on fixed
   `generic_iteration`, with pooled medians of 15,367.787 versus 14,112.992 ns/unit and a conservative nominal
   95% interval of -10.2126% to -5.9577%. All 808 formal samples matched; all 19 timed legs and all 11 final
   admitted idle windows passed their host gates. Run `20260824T022115Z-64b2afe9` confirmed key/Droplet cleanup.
   The follow-on C2.2 slice is now implemented: a mode-selected whole-function plain executable tape
   replaces the Region/receipt production metadata. The loader predecodes each source PC into one verified
   private operation (register/cell bookkeeping, primitive constant loads, guarded numeric
   arithmetic/negation/comparisons, guarded dense-array access, direct branch targets, non-HotIR numeric
   `ForLoop` backedges, and the accepted fused four/six-operation receipts) or an effect boundary; run
   suffixes group effect-terminated blocks under one deterministic-GC admission, plain mode is selected once
   at function entry, metered/instrumented frames stay scalar, and guard misses side-exit to the exact
   canonical instruction before any mutation. Scalar-versus-tape differentials, the release preflight,
   conformance verification, fuzz smoke, and native/wasm memory-limit gates passed, and its controlled
   DigitalOcean gate accepted exact candidate `77e0d92` against the receipt baseline `84e732a`:
   4.0059% lower `generic_iteration` ns/unit (pooled medians 13,092.320 versus 12,567.856) with a
   conservative nominal 95% interval of -7.1417% to -2.8710%, excluding zero and the +5% boundary. All
   808 formal samples matched the checksum and run `20260824T053606Z-bad0cf1d` confirmed key/Droplet
   cleanup. Same-day gates for the other four tracked workloads were host-limited: one qualified
   `dense_tables` comparison was inconclusive by the predeclared conservative-envelope rule (+0.881%
   point, -3.046% to +6.830%) and every other attempt failed host qualification on both AMS3 and NYC3
   pools; diagnostics were retained, nothing else was published, and all eight Droplets/keys were
   confirmed deleted. Next come compiler liveness/receipts, a VM-owned register arena with
   compact iterative frames, and direct table/call lanes before productionizing type-directed wasm/AOT.
   The C2.3 preparation slice is also merged:
   `rustia_vm::instruction_facts` (exhaustive static operand/effect model for all 131 opcodes) and
   `rustia_vm::liveness` (per-function basic blocks, register liveness, cell may-attach state, captured-detach
   versus pure-cleanup clear classification, diagnostics, and the `BlockReceipt`/`LogicalReceipts` tape
   contract), plus the default-off `VmSettings::enable_opcode_census` exact scalar opcode counter and the
   generated five-workload census `design/58-vm-core2-c23-census.json` (Move+ClearLocalCell are 47-63% of
   executed instructions; 195 of 210 static clears are receipt-gated removable pure cleanup). Default
   bytecode, fuel, and deterministic collection are unchanged; classification is pinned by execution
   differentials. The C2.3 logical receipts are now connected to the tape executor: optimized programs carry a
   validated `ProgramLogicalUnits` schedule and the VM charges fuel, `executed_instructions`, and
   deterministic-collection admission by those units on both the scalar dispatcher and the plain-mode tape
   (HotIR and leaf-call plans are skipped only in a prototype whose own slice of the schedule is
   non-identity, and the `Call` fast path charges the `Call`'s own units on top of a plan's fixed charge —
   per prototype since 2026-09-15), with equivalence pinned per family by
   minimum-fuel equality, per-prefix fuel sweeps, logical-clock equality, deterministic-GC interval sweeps
   that compare the collection count and next boundary, and the lattice invariant
   `deterministic_collection_count == floor(executed / interval)` on the tracked workloads across consecutive
   calls on both profiles (a multi-unit charge fires one collection per crossed boundary; the equality test
   that had silently disarmed the boundary for the rest of the VM's life was fixed 2026-09-15). The
   default-disabled O2 families are threaded through the full conformance corpus
   (`run_repository_with_experimental_o2`); the reachability-exact track — constant-branch folding,
   commit-late compound/self-update assignment, provably-nil tail folds, slot tightening — passed the then-712
   fixtures per family and together (0.1-11.0% executed-instruction reduction on the tracked workloads),
   while standalone dead-clear elision (24.7-35.3% per workload; most of the measured win) diverges on the
   upstream weak-root-liveness fixture and is hard-blocked until C2.4 frame root scans mask liveness-dead
   registers; a self-documenting ignored conformance test pins the divergence. Default compilation remains
   byte- and schedule-identical; enabling a corpus-green family is now purely a measured performance decision
   (see [`58-vm-core2.md`](58-vm-core2.md) §C2.3), and that decision is a **per-workload executor trade**
   only where the rewrite lands in a hot prototype: a prototype whose own slice of the logical-unit
   schedule is non-identity loses HotIR and its leaf-call plan (per prototype since 2026-09-15; the earlier
   program-wide gate is what made the 2026-09-13 host measurement 2.9x/2.1x slower on `closures_upvalues`
   and `generic_iteration`), so `dense_tables` — whose rewrite sits inside its own HotIR loop — swaps 16,298
   HotIR iterations for 99.75% tape coverage while `closures_upvalues` and `generic_iteration` keep their
   leaf plans with a census identical to all-off. Those are counts, not timings: the fixed tree has not
   been measured on the owned host yet (§C2.3 item 4), so the default stays off. The default-off
   `executor-diagnostics` feature and
   `crates/rustia-compiler/tests/executor_ownership.rs` report that ownership per workload on both
   leaf-call paths — the `Call` opcode's inline cache and the `call_value` replays that generic-for
   iterator steps take, so `generic_iteration` is seen serving 1,055 `__iter` replays per 32 units on
   both sides of the reachability-exact schedule; the same report shows
   `sparse_tables` taking 8,190 tape guard exits per 32 units — one per scattered-integer index
   operation — independent of O2. The C2.5a slice is also merged: numeric writes to a
   proven existing, non-nil array slot mutate in place, skipping `mutate_table_data`'s before/after memory
   census and its no-op `side_usage` adjustment while keeping readonly, `__newindex`, insertion-order, and
   the single `mutation_version` bump exact; a `debug_assert!` pins the shadowing invariant across the unit,
   conformance, and fuzz suites, and a Log/PUC differential matrix plus end-to-end Luau tests cover it. Its
   default-off `table-write-diagnostics` census showed the lane was then reached **zero times by all 17
   then-runnable benchmark workloads** — every numeric array-slot write in the suite was an append or a
   deferral, because each workload rebuilds its tables from `{}` over an ascending index. That result is what
   motivated adding `array_mutation`, which reaches the lane on 98.46% of its writes. Three exact DigitalOcean gates on fixed
   `dense_tables` work therefore measured only the guard's miss cost: `92d30b8`→`066ce1f` at +0.6165%
   (conservative nominal 95% interval -0.1571% to +2.2646%), `92d30b8`→`2082436` at -0.4457% (-2.5751% to
   +1.5953%), and, after the C2.2 tape landed and rebased the first two candidates away, `b253e9f`→`0a31255`
   at -1.8842% (-18.1155% to +0.3730%, wide because two of four baseline legs ran about 20% slow). All three
   have intervals containing zero, so they establish only that the guard costs nothing where it never fires.
   The new `array_mutation` workload then measured the lane where it does fire (98.46% of its numeric
   array-slot writes): control `7e6e71d` is candidate `accca92` with only the lane's two admissions removed,
   and on fixed `n=4000` work at 201 repetitions per leg the candidate used **8.2459% less time, conservative
   nominal 95% interval -12.8420% to -3.1921%**, excluding both zero and the +5% boundary. So the lane is
   free where it never fires and worth about 8% where it does; this is a per-workload result on one Linux
   protocol, `array_mutation` sits outside the public five-workload cohort, and public benchmark history is
   unchanged. Seventeen Droplets produced the four accepted gates, all confirmed deleted with their temporary
   keys, for about $0.51. Raising repetitions 101 → 201 moved the within-order qualification spread from
   5.098%-6.689% to 0.2412%/0.7950% and qualified on the first draw. `just bench-do-sweep` now reclaims any
   Droplet a run leaves billing and runs before every `bench`, after an API outage left one billing for about
   1.4 hours. The measurable dense-table target is now array append/growth and capacity prediction — a
   separate slice with its own gate (see [`58-vm-core2.md`](58-vm-core2.md) §C2.5). That slice, C2.5b, is
   now merged and measured: the append lane collapses the generic resize/overwrite/`remove_hash_key`
   sequence to one `Vec::push` with incremental rather than skipped accounting, `dense_tables` takes it on
   all 16,384 of its numeric array-slot writes, and an adversarial pass found no defect across a
   480,000-operation interleaved differential stress run. **DigitalOcean is retired**: six consecutive
   `dense_tables` gates failed host qualification there, and a controlled local test disproved the
   workload-instability explanation (quiet-machine spread was 8.65% for `dense_tables` against 10.56% for
   `array_mutation`), leaving host contention as the cause. Measurement now runs on the dedicated
   `ci-bench` host (Ryzen 5 3600X, SMT off, `isolcpus=4,5`), which measures **0.52% between-leg spread**
   against 15.12% on the retired sfo3 pool. Control `4e5eb70` versus candidate `eaf3917` on fixed
   `dense_tables` work at n=1500 measured **-1.7894%, conservative nominal 95% interval -2.1250% to
   -1.0770%** — the interval excludes zero, so this is an accepted speedup rather than a neutral result.
   The run used a simplified protocol without the paired qualification, idle gate or cooldowns; those
   should be wired into a runner workflow before it becomes the standing protocol.
4. **Advance the `bench/` crate into the standalone OSS benchmark project** — [`80-bench-suite.md`](80-bench-suite.md).
   The runner already exposes the requested engine matrix, five default workloads, 19 workload entries including
   source-covered `string_building` and six memory probes, a coherent Linux-only JSONL result snapshot, package metadata, split notes,
   a standalone HTML report, an explicit closed wasm pre-lock gate artifact, a wasm target bench for rustia+koto+rhai, a `call_ref`
   versus `br_table` dispatch microbench, and an explicit dedicated-Linode native-plus-wasm matrix lifecycle.
   Public history is now reconstructed end to end on the dedicated host: 75 revisions spanning 2026-07-03 to
   2026-08-26, five workloads each, one fixed harness (`daaf422`) and one pinned dependency set overlaid onto
   every revision, all 375 rows labelled `host: ci-bench`. Every revision produces byte-identical workload
   checksums, so the series measures speed alone; the geometric mean across the five workloads improves
   **4.291x (+329%)**, from 0.066x to 0.285x of mlua/Luau/JIT. Four revision pairs compiled to byte-identical
   binaries; across the 20 resulting comparisons their independent timings agree to **0.41% median and 5.74%
   worst**, which still bounds run-to-run variation far below the effect. That tail must be quoted per workload
   rather than as one number: spread scales inversely with per-unit cost, from 0.32% worst on `dense_tables` at
   39us/unit to 5.74% on `closures_upvalues` at 372ns/unit, where a 21ns difference is 5.7%. An earlier,
   smaller sample of 15 comparisons gave 0.50%/1.04%, and that worst case was published as a host property; it
   was not, it was which pairs happened to appear. The figure belongs to the backfill protocol's five
   repetitions, not to the machine — the A/B lane runs 101-201 and measured an interval about one percentage
   point wide on this host. The earliest engine-bearing commit (`8c0211d`) needed a declared workspace
   member and the `pub fn call_function` wrapper its successor added; both are recorded in the artifact's
   `source_modifications`, and its checksums match the other 74 exactly. Commits before it contain no
   `crates/` tree at all, so the series starts at the first commit that has an engine. The chart shows this
   series alone: the superseded Linode reconstruction and the recorded matrix line were removed from it
   because they were measured on other hosts, and mixing hosts lets a machine difference read as a
   performance change. Recorded matrix runs remain in `bench/history/runs.jsonl` and still supply the 1x
   reference. `just bench-history` now reproduces the whole series on the owned host from a clean
   checkout, which is what a change of benchmark machine requires: numbers measured on the old box are
   not comparable with the new one's, so the response to new hardware is to re-derive rather than append.
   The runner attests the machine before building, takes an exclusive lock, applies declared source
   repairs by re-deriving them from Git, and records the identical-binary repeatability check in evidence.
   Every commit landing on `main` now starts an automatic measurement on the owned host under that same
   protocol, appended to the published series, so the chart advances rather than freezing at the last
   reconstructed revision; a commit whose binary is byte-identical records nothing. The weekly matrix's
   publication merge is now seen by that lane like any other commit, because the app token that makes it
   does start push workflows; the byte-identical gate records nothing for it, at the cost of one build.
   A run that fails or is cancelled records no point for its commit. Runner policy is
   split into classes: the shared pool may not produce a timing and holds no `measurement-` role, at most
   one class may be authoritative, and measurement is reachable from a workflow only through the
   entrypoint that attests the machine first.
   Pull requests are screened on the same host against the merge base of the pull request and its
   target — not main's tip, which would attribute an unrelated commit landing mid-review to the change
   — over two passes with the order reversed in the second, so the sides interleave A B B A. The lane
   reports in the job summary and records nothing: no row, no published artifact, no write access to
   either repository, each pinned by `checkPrBench`. Its thresholds are measured rather than declared:
   `just bench-calibrate` measures one revision four times and compares it against itself under all
   three balanced splits of those passes, both sides of every split being the same binary, so each
   interval is the host's own spread and each threshold is the widest end of the widest split. The
   committed values are 1.20% to 7.60% per workload, calibrated on 2026-09-20 with the timed cores held
   at the `performance` governor (the earlier 1.61% to 3.88% set was measured under `schedutil`). They
   are not all above the 1.5% this lane was planned around: `sparse_tables` sits below it and
   `generic_iteration` is five times above it, widened by one slow leg in the calibration — which is why
   they had to be measured. There is no default, because a declared figure sitting inside that spread
   would report movement on changes that did nothing.
   The A/A run doubles as the protocol's canary and found real drift, twice. `sparse_tables` read 1.2%
   faster than itself, and on a later run `stdlib_math_table` read slower, both under the split that
   places each candidate pass immediately after its baseline, both excluding zero. That is monotonic
   session drift rather than a code fault, and it is precisely what the lane's A B B A ordering cancels
   — which turns that ordering from a stated precaution into a measured requirement. The canary
   therefore fails only on the straddling split, the arrangement whose indistinguishability the lane
   actually claims; the drift-sensitive splits still widen the thresholds and are recorded per workload.
   The analyzer gained the two readings the
   lane needs: `comparison.direction` against zero, because a one-sided threshold reports a genuine
   speedup as `material_regression_excluded` — replaying the committed `84e732a` analysis shows an 8.2%
   improvement published under exactly that wording — and `--confidence`, because five workloads screened
   at 95% give roughly a 23% chance one moves on a null change and more repetitions cannot fix a per-test
   error rate. The `*_ci_95_percent` fields stay at 95% whatever level is asked for, verified by
   reproducing a months-old committed analysis byte-identically. The fork guard is already in place rather
   than deferred: `pr-bench.yml` refuses pull requests from forks, which changes nothing while the
   repository is private and is the guarantee that matters the day it is not.
   Next work is the actual standalone-project split, the interop/serde/fuel-limit workload tranche, and
   regression alerts.
5. **Stabilize and publish the public crates.** Settle the consumer API and semver surface, close the generated
   release gates, flip `publish = false` in dependency order, and publish honest per-workload documentation.
6. **Keep the executable conformance corpus green**. The current repo has 723 executable fixtures covering 517
   behavior refs with no pending rows: 705/499 are Luau 0.709 oracle-backed and 18/18 are offline PUC 5.1.5
   oracle-backed. New engine work should add focused fixtures rather than re-opening pending inventory.
7. **Maintain benchmark operations** — one owned, attested machine times everything; the shared pool times
   nothing. `ci-bench` is the sole timing authority, qualified by properties that are checked rather than
   asserted: cores held off the scheduler, SMT disabled, each pinned core held at its ceiling with boost off,
   timed work pinned to one isolated core, one job on
   the box at a time, and every run attested against the frozen config before it builds. Each of those properties is a
   key of the `qualification` record in `bench/operations-policy.json`, which the status renderer refuses to
   drop one of, so the record and this list do not drift apart. The ceiling is read from the cpufreq policy the
   kernel publishes: a pinned core
   exposing none is accepted as unscaled, and `just bench-host-check` prints each isolated core's governor,
   range and boost so that reading is available before a run rather than only from a refusal after one. Every workflow that
   measures reaches it through `tools/run-dedicated-benchmark.js`: the cross-engine matrix (`matrix`), the
   per-commit series (`main`), pull-request screening (`pr`, plus its `live-serde` and `calibrate` jobs),
   dispatch-only A/B acceptance (`accept`, `.github/workflows/acceptance-bench.yml`: two to sixteen legs, a
   summary only, recording nothing), and the memory cohort (`memory`). The reconstruction (`history`) uses the
   same entrypoint but is operator-run rather than workflow-triggered.
   `.github/workflows/weekly-matrix-bench.yml` measures all thirteen engines on the owned host and lands
   accepted cohorts on `main` through a pull request it opens and merges itself with a merge commit
   (`tools/land-matrix-publication.js`; `main` takes nothing but pull requests, and this is the
   repository's single sanctioned CI contents-write grant, shape-pinned by `tools/check-workflows.js`).
   The publication commit carries `[skip ci]`, so the pull request starts no checks. The merge is made as
   the `slowchop-ops-controller` app rather than with the workflow token, because a named app can be named
   as a ruleset bypass actor and `github-actions[bot]` cannot: that is what allows `main` to require status
   checks while a publication pull request still shows none. `main` requires eleven checks through a
   repository ruleset whose only bypass actor is that app. An app-token merge starts push workflows, so the
   dispatches after landing may duplicate runs the merge already made, and they are kept until a landing
   has shown which. A publication pull request must land as a merge commit, now the only method the
   repository enables. It runs weekly (Sunday 15:00 UTC, plus manual dispatch) and unconditionally rather than gated on pending commits: the gate asked
   the Rustia-progress question, which the per-commit series now answers directly, while an ungated run also
   makes host drift — kernel, microcode, thermal — visible instead of hiding it. Its first cohort
   (`f480758`) landed by direct push, and the second (`f49646f`, 2026-09-15) was the first to land
   through the pull request, joined by `7826508f` (2026-09-20) and `a5cd40cb` (2026-09-21), so
   `bench/history/runs.jsonl` now holds only those four cohorts of 65 rows, every one measured
   on `ci-bench`: the 1,230 rows measured on retired cloud hosts were removed once every generated report
   reproduced byte-identically without them. `bench/evidence/matrix/f480758.../host-bridge.json` records how
   the two machines compared first, because that could not be recovered afterwards and is not a constant —
   the ratio ranges 0.71x to 1.86x by engine, so absolute nanoseconds do not carry across the change while
   the published same-run ratios do. Every emitted row now names its machine in a `host` field
   — provenance published in row data, never an address — and the single-host rule is mechanical: the
   acceptance rules recompute the expected label from the frozen config, and `tools/check-benchmark-results.js`,
   `tools/merge-rustia-ci-memory-data.js`, `bench/publish-memory-analysis.js` and the renderer each refuse a
   table whose rows disagree rather than labelling the mix, because every figure on that page is a ratio
   between two rows. That closes the way the memory table silently mixed hosts on 2026-09-05, when a
   rustia-only refresh left 99 six-week-old cloud rows beside 11 new ones under metadata naming one host
   (`81-benchmark-operations.md`, "Row provenance"). The memory merge also refuses a generated cohort whose
   rows, `metadata.host` or history line disagree, and, because it is per engine, a cross-engine cohort
   replaces every engine it measured whatever machine the stored rows name, which is how a table is
   re-measured rather than refused. The rule reaches the memory table and each memory cohort in the checker,
   the renderer pins the timing table, the memory table, and the memory history's latest cohort to the timing
   authority itself, and the history charts read the field: the recorded timing chart draws only the
   reference machine's rows, a reconstruction from another machine is refused, and the memory history draws
   one machine against a reference from that same machine. A label may contain only `[A-Za-z0-9_-]{1,128}`, so
   it can never be a dotted name or an address, and the checker's self-test holds the thirteen scripts that
   emit or validate one — the frozen-config checker, the memory publisher and merge, the operations-status
   renderer and the host-bridge recorder among them — and the harness's `--host` to that one literal; the
   owned-host coordinator refuses an ssh destination whose host is a published label, and refuses matrix
   evidence whose uname is not `uname -srvm` by its form rather than by a name. The published data
   repository's memory table and history were stamped the same way (slowchop/rustia-bench-data PR #9).
   Nothing in this repository can start a billed instance, on a
   timer or otherwise. The memory cohort was the last lane that could, and only by explicit dispatch; it now
   runs on the owned host too (`.github/workflows/rustia-memory.yml`), pinning two isolated cores rather than
   one because the supervisor samples while the benchmark runs. That lane has completed there: on 2026-09-05 it
   produced the rustia-only cohort `rustia-ci-070c387b173c` at `070c387`, 11 rustia rows from 99 raw samples,
   which is now the whole of `bench/memory`. Per-VM memory is measured inside a dedicated cgroup, and the
   host's unprivileged CI user cannot write a cgroup-v2 subtree of its own, so the lane needs either root or a
   systemd-delegated subtree containing the job's own process (`Delegate=memory` plus `DelegateSubgroup=` on
   the unit that runs the runner, without `ProtectControlGroups=`) — the delegation the host now provides;
   the tools find it by walking up from the process's own cgroup, check it before anything is cloned, and
   name which fault is present.
   The check is deliberately not relaxed: the cgroup is what the per-VM figure is measured in, so a run
   without one would produce numbers that look like measurements and are not. The lane measures one of two
   cohorts chosen at dispatch: `rustia-only`, the routine 11-row per-commit cohort, and `cross-engine`, the
   110-row thirteen-engine cohort under the same protocol, which is what returns the memory comparison and the
   mlua/Luau/JIT reference from the machine the Rustia rows come from. In the published data each engine's
   rows come from the newest cohort that measured it. The measured commit's own tools measure, but the merge
   into the data repository and the push run main's copies, taken before the checkout detaches, and a commit
   whose `bench/analyze-memory-samples.js` lacks the host-label line — one from before a row's host was confined
   to a label — is refused before the build. No cross-engine cohort has run on `ci-bench` yet, so the comparison
   stays pending until the first is dispatched. Three cloud coordinators are deleted and the eleven Linode
   lifecycle recipes with them; `tools/run-linode-benchmark.js` survives only as the home of the matrix and
   memory acceptance rules the owned-host coordinator borrows — nothing runs its lifecycle, and release
   preflight runs only its offline self-test. The rented-model A/B runner, `tools/run-linux-fixed-benchmark.js`,
   is deleted; the frozen-config validation it carried lives in `tools/check-dedicated-config.js`, which also
   refuses a `provider.host.name` that is not a label. The ledger-recovery tool,
   `tools/recover-linode-linux-history.js`, and its `just bench-recover-linux-history` recipe are deleted
   with them: it rebuilt the ledger into 905 rows from the retired hosts, none naming a `host`, so after the
   retirement it could only replace the good file with one the checker refuses. Re-deriving the series for a
   new machine is `just bench-history`; removing a cohort is `tools/retire-cloud-history.js`.

**Pre-release gates:** see the "What's next" section below. Object reclamation/limits and deterministic
decision reset, maintained live-memory accounting, and the shared native/Node-V8 mixed-heap pressure gate are
implemented. Benchmark publication and the `LuauIntegerType2` watch remain ongoing gates.

**Project setup facts:** dual `MIT OR Apache-2.0` (holder Gak Dev Pty Ltd); the public/private boundary +
`private/` mount are in [`AGENTS.md`](../AGENTS.md); no Linear.

## Canonical claims (corrected — supersede any stronger wording elsewhere)
- **Perf — state it PER-WORKLOAD, not as a blanket "~5× koto"** (review H2). The historical 11-workload seed
  found reference Luau **~4.7–9× faster than koto** on tables/closures/coroutines/stdlib-math, approximately at
  parity on generic iteration, and slower than koto on pcall/xpcall; those results set must-match workloads, not
  a rustia claim. In the current five-workload Linux snapshot, rustia trails `mlua-luau` by 2.615x-5.880x by
  minimum time with a 3.944x geometric mean, and the generated gate approves no blanket speed headline. Two distinct historical
  koto multipliers remain useful context: the **float-path ~5–9×** (measured) and the
  **deterministic-path ~30–40×** (the sim's *real* incumbent — host fxmul/curve, per `ai_bakeoff`). The fused-op
  sub-µs (~0.2 µs) is **bespoke/private-fork, kernel-specific**. The committed benchmark crate now has five
  default measured workloads plus a source-covered #15 `string_building` workload; rustia's public numbers must
  earn approval from the existing generated gate before claims.
- **wasm cross-engine PRE-LOCK GATE is CLOSED:** production rustia wasm is now measured on V8 (node/deno),
  Bun's JavaScriptCore-family runtime, and SpiderMonkey, and the `call_ref` versus `br_table` dispatch
  microbench is measured on those same families. `br_table` remains the portable dispatch baseline. Real
  Safari/iOS JSC correctness/golden coverage remains an open public-release gate; active timings stay on Linux.
  Wasm-3.0 pass done: `br_table` dispatch stays (tail-calls/`call_ref` don't reopen it — `call_ref` is a cheap
  P1 micro-bench), **wasm-GC rejected** (determinism/layout), relaxed-SIMD stays banned.
- **You CAN JIT in the browser — VALIDATED (spike `65-wasmjit`, verified).** Not native machine code, but you
  emit **wasm** at runtime and V8 compiles it. Measured: a hot kernel emitted to wasm + instantiated runs at
  **~4.6 ns/op = ~176× the interpreter** (within 1.35× of native), **~4.7 µs standup → crossover ≈ 6 calls**;
  shared-heap (imported VM memory) had no penalty. ⇒ wasm-emit JIT/AOT is now a **validated tier**, not a
  research option — the in-browser route to near-native speed. Caveat: monomorphic-f64 kernel; general Luau
  needs **type guards + deopt**, so gate on a hotness counter and use **type-directed compilation** to emit
  guard-free specialized wasm. (Corrects the earlier sloppy "JIT moot in-browser.")
- **SotA performance plan is no-unsafe:** [`56-performance-research.md`](56-performance-research.md) ranks the
  current speed work. Compact deterministic table/atom indices, compact site-scoped live-slot caches,
  mutation-safe `GETIMPORT`, guarded generic fixed-result `FASTCALL`, constant-key
  `GETTABLEKS`/`SETTABLEKS`, two-lane live `NAMECALL` lookup, shared compiled constant storage, and 13-byte inline
  literal/runtime strings are landed. Exact dedicated-Linode evidence now measures 2.42% lower ns/unit for the
  corrective dense candidate, with a conservative 95% interval of -3.35% to -1.57%, and 65.959% lower ns/unit
  for the complete OOP candidate on `oop_metamethods`, with a conservative interval of -66.39% to -65.55%; the latter also includes the
  weak-table GC-scan correction, and both comparisons retain
  three final-candidate history rows. A separate exact 512-VM sampler confirms the shared/inline representation's
  per-VM constant-string savings across 16 paired rounds while exposing the small one-time program-image and
  construction tradeoffs. The first accepted exact cross-engine matrix reduced rustia's median ns/unit by
  22.08%-34.97% across all five tracked workloads versus the previous committed Linux snapshot. The following
  numeric-sort matrix improved dense median time another 8.90% and reduced that workload's `mlua-luau` gap from
  9.20x to 8.54x. The landed private HotIR then retained native aggregate performance on its exact Linux gate
  (+0.47% normalized geometric mean versus the preceding snapshot). The subsequent prototype-polymorphic
  closure-call/direct-immutable-upvalue slice reduced `closures_upvalues` Linux median latency by 15.036%
  (270.427 to 229.766 ns/call). The cached length-border slice was then exactly gated, measured no win on
  either length-heavy workload (+1.76% dense and +0.62% sparse candidate deltas), and was reverted; its
  spare-capacity mutation-version correctness fix survives. The current generated minimum-time
  geometric-mean gaps are 3.944x against `mlua-luau` and 4.336x against Luaur; rustia beats Ruau on three of
  five tracked workloads by minimum time. The local -29.31% dense result did not reproduce on Linux and remains directional only. These are workload- and
  protocol-specific results, not blanket speed or RSS claims. Next come bounded object reclamation/quotas,
  broader long-run history, Linux-specific profiling of HotIR, verified multi-effect/control-flow HotIR growth,
  superinstructions/O2, and type-directed wasm/AOT. Native copy-and-patch/JIT
  work remains a research reference unless it can satisfy the public
  `unsafe_code = "forbid"` gate.
- **Luau target PINNED** (review C1): **Luau 0.709** (= our mlua oracle), standalone (not-Roblox) surface,
  vector width 3, flags-off — see [`05-luau-target.md`](05-luau-target.md). **f64-only is VERIFIED correct**
  (`math.type==nil`; integers are default-OFF even at HEAD 0.727 behind `FFlag::LuauIntegerType2`).
- **Conformance corpus is executable and pending-free** (review C2 follow-through): the runner reports
  **726 fixtures**, **520 refs**, 708 of them backed by the Luau 0.709 oracle and 18 by the offline PUC
  Lua 5.1.5 oracle, **0 pending-oracle**,
  **0 pending-impl**, and **0 rustia-only**. That named executable denominator is authoritative; it does not
  claim a one-to-one audit of every row in the older 709-row design inventory. Future work is focused oracle
  coverage and reconciliation for newly discovered gaps, not treating old model prose as implementation truth.

## Determinism model (one engine, three determinism profiles plus Puc51Compat)
- **LuauCompat** — a real `Profile` variant that currently selects Luau-style hash iteration and exposes the
  Standard service/capability surface. Further reference-oriented knobs require explicit implementation/tests.
- **Puc51Compat** (added 2026-07-31) — PUC Lua 5.1.5 source-compat profile: faithful `ltable.c` table layout
  and `next()` order, PUC `auxsort`, `%.14g` number formatting, constructor pre-sizing, and PUC constructor
  write order, with ReproducibleStandard host-service rules. Orthogonal to the determinism axis below;
  parity is pinned by the offline-derived `conformance/fixtures/puc51-compat/` corpus (see
  [`50-conformance.md`](50-conformance.md)).
- **ReproducibleStandard** (the recommended public default) — targets the full pinned standalone Luau surface
  and is pending-free on the current 520-ref denominator, with reproducible choices for unspecified behavior
  (insertion-order iteration, pinned PCG32, Schubfach tostring, libm). Its `os` table is always registered;
  without providers, `os.time`/`os.date` read the pinned epoch and `os.clock()` returns the deterministic
  logical clock (executed instructions / 1_000_000) instead of the former `0.0`
  ([`72-host-services.md`](72-host-services.md)). It is not an unbounded full-conformance or
  byte-identical-to-reference claim; semantic-tail and full-target gates remain.
- **DeterministicSim** (grabby) — ReproducibleStandard minus the env-dependent globals `collectgarbage`,
  `gcinfo`, and `os.*`, plus any optional host-supplied `loadstring`; it rejects external-mutable-state and
  nondeterministic-effect host registrations. Its typed `HostSchema` registrations can access an invocation's
  deterministic input, slots, effect journal, and slot-write journal only through the matching capability view.
  The invocation makes fuel/OOM/record overflow/yield-boundary failure/missing dispatch/host callback panic
  hard, shares fuel across all nested calls, and publishes bounded records only after the outer operation
  succeeds. Deterministic
  output/clock/entropy providers are admitted: a stdout
  sink may be attached on any profile, the `os` table (all four members) is registered iff at least one
  deterministic clock provider is plugged, and replay-stable entropy may select the initial random sequence.
  A nondeterministic input provider is rejected by the facade `build()` and silently dropped by the infallible
  VM constructors. Weak tables remain supported and are
  swept on the deterministic
  instruction-count schedule. A sandboxed subset designed for bit-identical replay and verified on the current
  tested matrix. (M2: the Standard modes use an
  **incremental/paced** collection schedule; only DeterministicSim
  uses the full-collect-on-instruction-count trigger.) Scheduling and weak sweeping are implemented; complete
  ordinary-object/program reclamation, reuse, telemetry, limits, maintained quota accounting, and script-visible
  memory counters are implemented; the maintained mixed-heap pressure gate is green, while interval tuning and
  external allocator/RSS history remain open.

`GuestSurface::HardenedLeaf` is a narrowing layer on `DeterministicSim`, not a fourth profile. It is the
authoritative stateless-action contract: sealed allowlisted globals/libraries/host namespaces, no ambient or
cross-tick guest authority, bytecode policy validation, mandatory memory limit, direct `CompiledEntry`
invocation, and a separately versioned guest ABI.

## Canonical doc map (status)
| doc | role | status |
|---|---|---|
| **CURRENT.md** (this) | the live canonical state | authoritative |
| [`78-host-functions.md`](78-host-functions.md) | shared synchronous callback definitions and reclaimable live functions | **implemented; controlled-client integrations verified** |
| [`02-pillars.md`](02-pillars.md) | project pillars, non-goals, and the rabbit-hole decision filter | **canonical** |
| [`30-architecture.md`](30-architecture.md) | the ADR (8 axes, decided) v3 plus v3.1-v3.3 refinements/corrections | authoritative for design |
| [`05-luau-target.md`](05-luau-target.md) | the pinned-target spec (Luau 0.709) | **authoritative** ✅ |
| [`50-conformance.md`](50-conformance.md), [`../conformance/COVERAGE.md`](../conformance/COVERAGE.md) | per-behavior conformance table + executable coverage notes | **current runner:** 726 fixtures / 520 refs / 0 pending |
| [`55-improvements.md`](55-improvements.md) | 10 adopted improvements (+ IC/hot-cold promoted) | authoritative |
| [`56-performance-research.md`](56-performance-research.md) | no-unsafe SotA performance stack, top-10 ordered bets, C++ Luau estimates | current research note |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | typed invocation plus the deterministic effect-queue seam to grabby's parallel/rollback sim | **authoritative + implemented generic seam** ✅ |
| [`72-host-services.md`](72-host-services.md) | host services bus: stdout, determinism-labeled clocks and entropy, native/WASI/browser adapters, facade presets, logical `os.clock` | **implemented** ✅ |
| [`75-embedded-api.md`](75-embedded-api.md) | Grabby-Compatible Embedded API: public facade + typed schema/context transaction | **transactional install, coherent execution API, and synchronous function construction implemented** ✅ |
| [`76-koto-migration.md`](76-koto-migration.md) | public migration guide from koto-shaped embeddings to the implemented rustia facade | **first slice** |
| [`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md) | frozen stateless authoritative guest surface, direct compiled entries, and fast isolated reset | **implemented v1** ✅ |
| [`80-bench-suite.md`](80-bench-suite.md) | continuous perf-bench suite + CI matrix design | **runner/adapters/history/target-first report/package metadata and dedicated Linode lifecycle implemented; standalone split pending** |
| [`90-release-readiness.md`](90-release-readiness.md) | public crate metadata, publish gates, benchmark/CI gates, and release process checklist | **metadata/checklist/changelog/preflight/CI gates started; publish=false remains** |
| [`91-semver-policy.md`](91-semver-policy.md) | public crate roles, stability tiers, release-line rules, and publish order | **policy started; 0.0.x remains unstable** |
| [`crate-metadata.json`](crate-metadata.json) | generated cargo metadata policy audit for all workspace packages | **current audit: 8 packages / metadata policy ok / publish=false** |
| [`package-files.json`](package-files.json) | generated package file-list review for all workspace packages | **current audit: 8 packages / 0 forbidden paths** |
| [`publishability.json`](publishability.json) | generated publishability blocker review for all workspace packages | **current audit: all 8 blocked by publish=false; internal dependency order recorded** |
| [`00-constraints.md`](00-constraints.md) | the contract + target matrix (§C0) | authoritative (target pinned to 0.709) |
| [`15-gapclose.md`](15-gapclose.md), [`10-landscape.md`](10-landscape.md) | earlier snapshots | **superseded** by 20/30/50/55 + this file |

## 3rd-party review (2026-06-30) — status tracker
All 8 findings assessed **VALID** and **actioned** (folded into the docs below).

| # | finding | status |
|---|---|---|
| C1 | pin the Luau target | **RESOLVED** → [`05-luau-target.md`](05-luau-target.md): Luau **0.709** (= our oracle), standalone surface, vector-3, flags-off; **f64-only verified** (`math.type==nil`); int64 is a tripwire on `FFlag::LuauIntegerType2`. |
| C2 | conformance table not trustworthy | **RESOLVED for current corpus** — runner reports 726 fixtures (708 Luau 0.709 oracle-backed, 18 offline PUC 5.1.5 oracle-backed), 520 refs, and 0 pending rows. Keep adding targeted oracle fixtures for new bugs/features. |
| H1 | version drift | **FIXED** (this file + banners). |
| H2 | "~5× koto" too narrow | **RESOLVED** — drop the blanket number; state **per-workload** (below). |
| H3 | wasm gate + Wasm 3.0 | **RESOLVED** — Wasm-3.0 assessed (br_table stays; wasm-GC rejected; relaxed-SIMD banned); V8 + Bun/JSC-family + SpiderMonkey production wasm and `call_ref` dispatch measured. |
| M1 | three named modes | **RESOLVED** — one engine with concrete LuauCompat/ReproducibleStandard(default)/DeterministicSim runtime profiles. |
| M2 | Standard incremental GC | **RESOLVED as a scheduling/profile decision** — paced/incremental for Standard, full-collect-on-instruction-count for DeterministicSim; maintained live accounting is implemented and final tuning remains open. |
| M3 | promote inline caches + hot/cold | **RESOLVED** — compact site-scoped live-slot caches plus `GETIMPORT`/`FASTCALL` and dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` landed; hot/cold remains P1. |

## What's next (the genuine remaining public-release gates)
The review is actioned (all 8 folded in above + into the docs). The honest residual open items, in order:
1. **Complete the ordered advanced API boundary:** the composable chunk builder is implemented on the
   graph-preserving live `Value`/`Table`/`Function` spine plus its globals/table constructors. Atomic
   installation, fresh dependency cohorts, host-owned replacement, live-value serde, and synchronous
   host-function construction are implemented, with controlled-client integrations verified. Settle the
   public settings/export and opaque-userdata contracts before freezing the facade. The
   worker-local typed invocation seam, hardened stateless leaf, direct compiled-entry path, binary/rich/cyclic
   live values, memory telemetry, and quota enforcement are implemented. Preserve the concise typed facade and
   apply determinism restrictions according to the selected builder: `live()` intentionally promises none.
2. **Tune the GC intervals against the landed mixed-heap evidence** (Deterministic instruction-count interval =
   determinism ABI; Standard paced-debt multiplier), without converting the correctness soak into a timing
   benchmark.
3. **Harden compiler/runtime error-phase parity** with focused oracle fixtures plus differential and longer
   coverage-guided fuzz campaigns; the stable all-opcode deterministic mutation baseline is implemented.
4. **Close the full correctness/build/golden target matrix.** Linux-only timings remain intentional, but public
   release still requires the native and browser targets in [`00-constraints.md`](00-constraints.md) to build
   and reproduce the declared goldens; real Firefox/Safari and non-Linux native coverage is still incomplete.
5. **Record broader production benchmark baselines** for rustia and each completed external adapter. The exact
   dense and OOP fixed-work gates, plus the string-focused 512-VM evidence gate, are committed and resolved for
   their declared Linux protocols. Continue using recorded fixed work, paired order, retained raw samples, and
   actual harness-produced final rows; extend the evidence to longer-run public history and realistic mixed heaps
   without generalizing these workload-specific results. Keep `RESULTS.md`, claims, and the static benchmark view
   consuming the same JSONL history.
6. **Stabilize and publish the crates** in the generated dependency order after release preflight is green.
7. **Watch `FFlag::LuauIntegerType2`** upstream (the f64-only tripwire).

These span the remaining engine hardening and product/release work. The architecture decisions are
complete, but implementation is not release-complete. The cross-engine, conformance, and benchmark gates are
already active. Fresh performance measurements use the owned `ci-bench` workflows;
local timings and the retired cloud lifecycle are not release evidence.
