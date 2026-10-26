# VM Core 2 — interpreter parity without a JIT

> **Status (2026-08-24): C2.1 was remotely falsified; C2.2 Region-integrated private receipts were
> remotely accepted with 8.1651% lower ns/unit; the C2.2 mode-selected whole-function plain tape is
> implemented, correctness-validated, and remotely accepted with 4.0059% lower `generic_iteration`
> ns/unit against the receipt baseline. On the owned host (2026-09-05 to 2026-09-12, three runs agreeing)
> the tape reads faster on `sparse_tables`, `closures_upvalues` and `stdlib_math_table`, about 4% slower on
> `generic_iteration`, and about 10% slower on `dense_tables`. The C2.3 reachability-exact track was measured
> as the default on the owned host on 2026-09-13 and refused: 2.9x and 2.1x slower on `closures_upvalues` and
> `generic_iteration` through what was then a program-wide executor gate, plus a determinism bug in the
> unit-charged collection lattice; both were fixed 2026-09-15 (the gate is per prototype, the lattice advances
> once per crossed boundary) and the re-measurement is pending.** This is the execution plan for the largest
> safe non-JIT performance programme. It refines the
> performance queue in [`56-performance-research.md`](56-performance-research.md); it does not change Luau
> semantics, the deterministic number contract, or the public no-unsafe policy.

## Why this is the next programme

The benchmarked `luaur` adapter is an interpreter. It does not enable Luaur's optional native-codegen feature,
yet it is materially faster than rustia on all five tracked workloads. The remaining gap is therefore not
explained by a hidden JIT. Local source comparison and Rustia's own accepted profiles point to three broad
differences:

1. Luaur executes from a compact stack/frame model through a thin dispatch loop. Rustia still creates separate
   register, cell, vararg, root, and debug state around calls and runs a policy ladder before semantic dispatch.
2. Rustia bytecode contains many compiler temporaries and explicit cleanup operations. A measured dynamic
   histogram found `Move` plus `ClearLocalCell` at 42–61% of instructions on four tracked workloads, with an
   even larger share in the generic-iteration trace. Making those operations cheaper produced an accepted
   8.45% improvement; grouping the same operations in Region Engine v1 was neutral because it still dispatched
   each operation internally.
3. Existing dense-table writes and several call paths still pay general accounting or result-packing costs even
   after their guards prove the common case.

Two tempting rewrites are explicitly not first:

- `Value` is already a sealed, copyable 16-byte tagged value, the same broad size class as Luaur's value.
- tables, functions, and upvalues already live in safe indexed `VmCell` side stores rather than in the old
  traced-arena shape described by earlier plans.

Rewriting either would spend risk without attacking the measured dispatch, compiler, frame, and table tolls.

## Target architecture

VM Core 2 remains one safe, portable interpreter. Public bytecode and the semantic dispatcher remain the
authority. Loader-built executable metadata may specialize verified shapes, but every guard or policy miss
resumes the exact source instruction.

The target has five layers:

1. **Executable-function plans.** Verify register bounds, control-flow targets, effects, and safepoints once
   when a program is loaded. Store compact basic-block plans and colocated cache descriptors beside immutable
   bytecode.
2. **Three coarse execution modes.** Select plain, metered, or instrumented execution at function entry rather
   than retesting fuel and coverage mode at every instruction. A block carries its logical cost and prefix
   costs. If fuel or deterministic GC lands inside it, execute the exact prefix through scalar semantics.
3. **Compiler O2 and liveness.** Lower directly into existing local registers where evaluation order permits,
   use commit-late assignments, add constant/RK forms, and replace temporary cleanup bytecode with verified
   liveness receipts. Captured locals retain explicit cell-close behavior.
4. **VM-owned register arena and compact iterative frames.** Use safe indices into reusable value/cell storage;
   push/pop compact frames instead of recursively constructing independent register vectors. Scan active frame
   windows directly for roots and materialize traceback strings only on demand.
5. **Direct table and call lanes.** Mutate proven existing dense slots without a full memory census, update
   accounting only on growth, colocate table/call guards with executable instructions, and write fixed-arity
   scalar call results directly into destination registers.

This is an interpreter design. It emits no native code, allocates no executable memory, and adds no `unsafe`.

## Non-negotiable invariants

- Luau 0.709 behavior, diagnostics, evaluation order, and metamethod timing do not change.
- Deterministic f64/libm behavior, NaN and negative-zero rules, insertion order, and profile restrictions remain
  exact.
- Fuel and `executed_instructions` count logical operations, not optimized host operations. A fused plan records
  the represented count. Compiler O2 changes must be declared as a compiler/fuel-schedule revision until block
  receipts make the schedule optimization-independent.
- A deterministic collection boundary inside a plan forces exact scalar-prefix execution; weak-table
  observability must match at every tested interval.
- Coverage and stepping use the scalar/instrumented mode and retain exact PCs.
- Allocation, guest calls, throw/yield points, metamethod entry, and host re-entry are explicit safepoints.
- Malformed public `Program` values keep the existing structured error and PC; loader recognition must decline
  rather than hide an invalid operand.
- Public workspace code remains under `unsafe_code = "forbid"`.

## Rollout

### C2.0 — freeze evidence and modes

- Keep the current interpreter as the differential oracle.
- Record static bytecode counts and effect boundaries for all five tracked workloads.
- Keep developer/self-hosted machines correctness-only. Controlled timings run only on operator-started
  DigitalOcean droplets, which must be deleted before evidence promotion.

### C2.1 — stop dispatching avoidable register work (implemented, falsified, removed)

- Loader-recognized `ClearSpan` plans cover monotonic contiguous `ClearLocalCell` runs in either compiler emission
  order. A two-byte-per-PC map is allocated only for qualifying functions and includes interior suffixes.
- One admitted plan clears the value and cell slices in bulk, then advances the original logical instruction
  count. Fuel, coverage, allocation debt, or an intervening deterministic collection boundary selects scalar
  execution.
- Region Engine receives loader barriers at ClearSpan entries, so Region, ClearSpan, and HotIR can coexist in
  one function without double execution or priority ambiguity.
- Loader-recognized `RegisterBlock` plans cover the exact compiler-emitted `Move target,lhs`; `Move temp,rhs`;
  guarded numeric-or-dense-`GetIndex`; `ClearLocalCell temp` shape. The executor snapshots both sources and
  checks every dynamic guard before mutation. Success commits only the scalar sequence's final target/temp
  state; failure resumes at the first `Move`, retaining generic/metamethod behavior and the operation's error PC.
- Public compiler bytecode is deliberately unchanged. An experimental direct-register lowering reduced static
  bytecode but changed deterministic collection cadence, so it was rejected until optimization-independent
  block receipts exist.
- Loaded-program memory admission constructs metadata once and charges its exact incremental logical-live size,
  including ClearSpan, RegisterBlock, and Region maps, before the program becomes visible.

Static analysis finds four eligible plans in `generic_iteration` (three `Add`, one `Mul`) and one `GetIndex`
plan in `dense_tables`. With current HotIR priority, generic iteration should enter RegisterBlock about
`69*n + 1` times (276,001 at the gate's `n=4000`), while HotIR consumes most dense-loop continuations. These are
coverage observations, not latency claims.

The exact same-host DigitalOcean gate compared baseline `9695a62` with candidate `2bb7bef` on
`generic_iteration` at fixed `n=4000`. All four qualification pairs passed with only 0.659% maximum within-order
ratio spread, and all checksums matched. Across the independent eight-leg comparison the candidate was 5.293%
slower; its conservative nominal 95% interval was +3.720% to +6.135%. Because that interval contains the
predeclared +5% material-regression boundary, the comparison was rejected and public benchmark history was not
changed. DigitalOcean run `20260824T003125Z-a92640e7` retained local diagnostics and confirmed both Droplet and
temporary-key cleanup.

This falsifies C2.1's separate-executor design. Baseline Region Engine already admitted the common six-operation
register lifetime as one run. C2.1 split it into a four-operation RegisterBlock plus another Region admission,
added loaded-program/prototype/map lookup at each block, and placed another policy ladder before the existing
Region path. ClearSpan was measured only as part of the same regressing candidate and is therefore not retained
without an independent winning gate. The public bytecode-shape tests remain useful for the replacement receipt;
the competing runtime tier, its barriers, and ClearSpan production metadata do not.

### C2.2 — make blocks the baseline executor (first receipt slice accepted)

- Introduce effect-terminated basic blocks with predecoded operands, logical cost, prefix costs, and liveness.
- Split entry into plain, metered, and instrumented modes while sharing semantic helpers.
- Move the common numeric/array lanes into direct block operations instead of nested Region dispatch.
- Require on/off differential tests for every opcode family, collection interval, fuel boundary, guard exit, and
  structured error PC.

The first independently measurable C2.2 slice reuses the existing Region admission boundary while changing its
inside representation. Private receipts are resolved once at function entry and may commit only while Region
already owns the complete source interval. The four-operation form represents the original expression result.
An exact six-operation form additionally absorbs the compiler's `Move destination,result`; `ClearLocalCell
result` lifetime tail, which covers the hot uncaptured-local accumulator updates while preserving the
destination cell link. Success advances the Region cursor by four or six logical instructions; a guard miss
executes the untouched canonical operations in order. This keeps surrounding register work in one uninterrupted
Region and removes the per-`Move` loaded-program lookup that falsified C2.1. It is a bridge to the full compact
tape, not another peer tier in the scalar dispatch ladder.

The compiler contract now pins both the four-operation expression prefix and
the normal-assignment six-operation lifetime tail. Focused differential tests,
independent semantic review, the workspace release preflight, conformance
package verification, fuzz tests, and native/wasm memory-limit checks passed
before the exact DigitalOcean A/B. No developer-host timing was used.

The frozen same-host comparison then tested baseline `9695a62` against exact
candidate `84e732a` on `generic_iteration` at fixed `n=4000`. Four
counterbalanced qualification pairs passed with 1.9286% maximum within-order
ratio spread. The independent eight-leg ABBA/BAAB comparison measured pooled
medians of 15,367.787 ns/unit for the baseline and 14,112.992 ns/unit for the
candidate: an 8.1651% reduction. Its conservative nominal 95% interval was
-10.2126% to -5.9577%, wholly below zero as well as the predeclared +5%
material-regression boundary. All 808 formal samples matched the checksum, all
19 timed legs and all 11 final admitted idle windows passed their host gates,
and three additional final-binary measurements were retained as evidence
sidecars without replacing the complete cross-engine matrix cohort.
DigitalOcean run `20260824T022115Z-64b2afe9` used an AMS3 Basic Intel
4-vCPU/8-GiB host and confirmed both temporary-key and Droplet deletion.
Its evidence was measured on that rented Droplet and was removed on
2026-09-13; the samples remain in Git history.

This result validates the architectural lesson from C2.1: keep verified work
inside one already-admitted executor instead of adding a competing per-PC tier.

The second C2.2 slice is the mode-selected whole-function plain executable
tape, now implemented. The loader predecodes every source PC into exactly one
private tape operation or an effect boundary: register moves and cell clears,
primitive constant loads with the value decoded once at load time, guarded
numeric arithmetic/negation/comparisons, truthiness inversion, guarded
dense-array reads and writes, direct `Jump`/`JumpIf`/`JumpIfNot` targets,
guarded numeric `ForLoop` backedges, and the accepted fused four/six-operation
register receipts. A per-PC run suffix groups execution into effect-terminated
blocks and doubles as the remaining logical cost, so one admission per block
replaces the per-instruction fuel/coverage/debt/GC ladder; a deterministic
collection boundary inside a block admits only the exact prefix and hands the
boundary instruction to scalar semantics. Execution mode is selected once at
function entry: plain (unmetered, uninstrumented) frames resolve the tape,
while metered, coverage, debugging, and stepping frames stay entirely on the
scalar dispatcher. A dynamic guard miss side-exits to the exact canonical
instruction before any mutation with its accounting already settled; build
verification declines malformed operands so structured errors keep their
current kind and PC. HotIR keeps sole ownership of its recognized `ForLoop`
backedges (they stay effect boundaries), and the closure-call PIC keeps `Call`
sites, so the plain path never consults competing executors at one PC. The
Region suffix map and the per-PC RegisterBlock map are retired; the receipt
recognizer and its atomic executor survive as fused tape operations resolved
at load time, which removes the per-`Move` map lookup entirely. Tape bytes are
charged exactly during loaded-program quota admission. Scalar-versus-tape
differential tests sweep every tape opcode family, branch edges, loops,
deterministic collection intervals, guard exits, malformed jump targets and
registers, and structured error PCs; the workspace release preflight,
conformance package verification, fuzz smoke suite, and native/wasm
memory-limit checks all passed with the tape as the production default.

The frozen same-host comparison tested the accepted receipt baseline `84e732a`
against exact tape candidate `77e0d92` on `generic_iteration` at fixed
`n=4000`. Four counterbalanced qualification pairs passed with 3.5815% maximum
within-order ratio spread (4.8139% raw cross-order, recorded for audit). The
independent eight-leg ABBA/BAAB comparison measured pooled medians of
13,092.320 ns/unit for the baseline and 12,567.856 ns/unit for the candidate:
a 4.0059% reduction with a conservative nominal 95% interval of -7.1417% to
-2.8710%, wholly below zero as well as the predeclared +5% material-regression
boundary. All 808 formal samples matched the checksum, all 19 timed legs
observed zero steal and passed their host gates, and 11 final admitted idle
windows passed. Three additional final-binary measurements were retained as
evidence sidecars without changing the complete cross-engine matrix cohort.
DigitalOcean run `20260824T053606Z-bad0cf1d` used an AMS3 Basic Intel
4-vCPU/8-GiB host for roughly 24 minutes and confirmed both temporary-key and
Droplet deletion. Its evidence was measured on that rented Droplet and was
removed on 2026-09-13; the samples remain in Git history. Combined
with the accepted receipt slice, plain-mode register/branch work now runs
about 12% below the pre-C2.2 baseline on this workload and protocol.

The same-day attempts to extend the tape gate to the other four tracked
workloads were host-limited and remain unpublished. One `dense_tables`
comparison qualified and completed but was inconclusive by the predeclared
rule: +0.881% candidate point estimate with a pooled interval of -0.631% to
+2.405%, while the conservative hierarchical envelope of -3.046% to +6.830%
touched the +5% material-regression boundary, so publication was refused and
diagnostics were retained. Every other attempt failed the predeclared host
qualification or block gates outright (within-order ratio spreads of 8.882%
to 23.389%, plus two contamination/steal discards) across both the AMS3 and
NYC3 Basic Intel pools as shared-host noise worsened through the morning. In
total eight ephemeral Droplets ran for about 131 minutes; every Droplet and
temporary SSH key deletion was confirmed, and cloud spend stayed under the
task's $1.00 ceiling. No workload showed a statistically clear slowdown, the
dense point estimate is consistent with neutrality on a HotIR-dominated
workload the tape barely touches (about 1% dynamic share), and the accepted
`generic_iteration` interval is wholly below zero, so the tape slice is
accepted on its diagnostic workload with the remaining per-workload gates
deferred to a quieter host window or the next full cross-engine matrix.

#### The tape on the owned host, all five workloads

The deferred per-workload gates were taken on `ci-bench` through the
acceptance lane (`run-dedicated-benchmark.js accept`;
[`81`](81-benchmark-operations.md)) three times: 2026-09-05, 2026-09-08 and
2026-09-12. Each run compared the receipt baseline `84e732a` against candidate
`141bdaa`, whose only change to `crates/` past the baseline is the tape commit
`77e0d92`; the seven other commits between them freeze gate configs and
publish evidence. Eight passes a side in A B B A order, five repetitions a
leg, on the fixed work the host template pins (`dense_tables` n=2,786,
`sparse_tables` n=1,449, `closures_upvalues` n=759,888, `generic_iteration`
n=9,969, `stdlib_math_table` n=107,256), read through the screening lane's
hierarchical bootstrap against its calibrated thresholds. Deltas are
candidate against baseline, intervals conservative nominal 95%:

| workload | run 1 | run 2 | run 3 | threshold | reading |
| --- | --- | --- | --- | --- | --- |
| `dense_tables` | +10.18% (+8.87..+12.58) | +10.52% (-17.66..+12.76) | +10.28% (+9.39..+13.09) | 1.68% | slower |
| `sparse_tables` | -5.26% (-7.04..-3.91) | -5.08% (-6.38..-3.01) | -4.95% (-6.27..-3.66) | 1.83% | faster |
| `closures_upvalues` | -22.73% (-24.69..-15.65) | -22.59% (-23.82..-17.71) | -23.75% (-25.66..-22.03) | 3.88% | faster |
| `generic_iteration` | +4.15% (+1.93..+5.51) | +4.30% (+3.26..+6.01) | +3.90% (+3.32..+6.26) | 1.61% | slower |
| `stdlib_math_table` | -8.38% (-10.04..-6.41) | -8.68% (-10.51..-7.07) | -7.38% (-8.27..-6.22) | 2.35% | faster |

Three readings stand out of this:

- `sparse_tables` did not regress. The tape is about 5% faster on it with all
  three intervals wholly below zero, which closes the question this campaign
  was opened on. `closures_upvalues` (about 23%) and `stdlib_math_table`
  (7-9%) are faster by the same standard.
- `dense_tables` is about 10% slower, in all three runs, and the two sides
  never overlap leg for leg: baseline legs read 37.6-39.2 thousand ns/unit,
  candidate legs 42.4-43.4 thousand. The reading is of the whole commit —
  the tape becoming the plain-mode executor, with the Region suffix map and
  the per-PC RegisterBlock map retired in the same change — not of the tape
  in isolation, and the mechanism has not been isolated on a workload the
  tape barely touches. Run 2's interval spans zero for a reason that is not
  the change: its first baseline leg read 65,325 because the whole leg ran at
  the isolated core's 2.2 GHz floor under the `schedutil` governor, and run
  3's first leg was caught the same way part-way through (median 66,026 over
  a 38,228 minimum). That is a host event, now attested against
  ([`81`](81-benchmark-operations.md)), and it does not move the point
  estimate. The O2 reachability-exact track (§C2.3) was measured as the
  default on the same lane on 2026-09-13 and refused; the re-measurement of
  the fixed tree is still pending.
- `generic_iteration` reads about 4% slower here, against the 4.0059% faster
  DigitalOcean reading above; the refused DigitalOcean `dense_tables` attempt
  (+0.881%, inconclusive) sits the same way against the 10% here. The two
  protocols share neither host (an owned Ryzen 3600X core against a shared
  Intel droplet), fixed work (n=9,969 against 4,000), nor candidate
  (`141bdaa` against `77e0d92`, which differ only under `bench/`), and neither
  has been re-run on the other's terms, so the disagreement is recorded, not
  adjudicated. The owned-host reading is the one the series tracks from here.

### C2.3 — complete compiler O2 and cleanup liveness (receipts connected; reachability-exact track corpus-green)

- Add commit-late local compound assignment, target-directed nested lowering, constant/RK arithmetic and index
  forms, branch folding, and local-slot tightening.
- Replace uncaptured temporary clears with liveness receipts; preserve explicit close/detach operations for
  captured locals.
- Make block logical receipts the optimization-independent fuel schedule before calling this phase complete.

The preparation slice is implemented and merged ahead of the whole-function tape:

- `rustia_vm::instruction_facts` is the exhaustive static operand/effect model for all 131 production
  opcodes (reads over-approximate, definite writes under-approximate, conservative read-all fallback for the
  exotic tail-call/expand shapes), and `rustia_vm::liveness` builds per-function basic blocks, backward
  register liveness, forward local-cell may-attach state, per-register classification (captured cells,
  uncaptured temporaries, values live across guest entries and block boundaries), and `ClearLocalCell`
  classification (required captured-local detach versus pure cleanup). Nothing on the execution path consults
  the analysis; default bytecode, fuel, and deterministic collection are unchanged.
- The tape-facing contract is `liveness::BlockReceipt` (per-block source interval, logical instruction count,
  live-in/out register and cell sets, terminator, guest-entry/allocation flags) plus
  `liveness::LogicalReceipts`: per-emitted-instruction logical units whose per-block sums restore the
  unoptimized fuel and deterministic-GC schedule over optimized code.
  `rustia_compiler::compile_with_options_and_receipts` returns the program together with that schedule
  (identity under the default compiler).
- `VmSettings::enable_opcode_census` (default off) counts executed instructions per opcode on the exact
  scalar interpreter, sharing the conformance-coverage admission gate so results and the fuel schedule stay
  identical. The generated `design/58-vm-core2-c23-census.json` pins the five-workload static/dynamic census:
  Move+ClearLocalCell are 47-63% of executed instructions, 195 of 210 static clears are
  removable-with-receipts pure cleanup, and none are captured detaches. Its byte-exact sync test keeps the
  report from drifting.
- Classification is validated by execution, not just construction: replacing every provably dead pure-cleanup
  clear with a value-preserving no-op leaves each workload checksum identical, while dropping required
  captured-local detaches observably breaks per-iteration capture identity.
- The optimization families — constant-branch folding, commit-late/direct compound and plain self-update
  assignment on uncaptured locals, reachability-exact dead Move/Clear tail folds, register-file tightening,
  and the separate standalone-clear elision — are default-disabled machinery with per-family differential
  tests, corpus-wide structural checks, and a generated workload preview
  (`design/58-vm-core2-c23-o2-preview.json`). Constant/RK operand emission is explicitly deferred: the census
  counts only 1-5 static candidates per tracked workload, far below the 33-69 removable clears, so spending
  new public opcode surface there is not yet measurement-justified; the analysis keeps counting candidates.

With the C2.2 tape merged, the logical receipts are now connected to the executor: optimized programs carry a
validated `rustia_vm::ProgramLogicalUnits` schedule (attached by `compile_with_options` whenever an O2 family
is enabled), and the VM charges fuel, `executed_instructions`, and deterministic-collection admission by those
units on both the scalar dispatcher and the plain-mode tape. Every folded instruction has fuel cost one, so an
instruction's restored cost is its own cost plus its extra units. HotIR and leaf-call plans assume one unit
per PC and are skipped only in a prototype whose own slice of the schedule is non-identity — HotIR keyed on
the prototype containing the loop, a leaf plan on the callee it replays, with the `Call` site charging its
own units on top of a plan's fixed charge (per prototype since 2026-09-15; see the enablement paragraph
below); deterministic collection boundaries stay on the original
count lattice but quantize to emitted-instruction edges inside a fused unit group: a unit charge that steps
over `k` boundaries fires the boundary collection `k` times at that instruction's pre-edge (iterations after
the first see identical VM state and are idempotent) and advances the lattice `k` steps, so
`deterministic_collection_count == floor(executed_instructions / deterministic_gc_interval)` and the next
boundary is `(floor(executed / interval) + 1) * interval` at every instruction edge, for every program,
across consecutive calls on one VM (`Vm::maybe_collect_with_roots` loops `while executed >= next`; fixed
2026-09-15 after the finding below). Unit-one programs reach a boundary only by equality, so default
compilation keeps its schedule bit for bit. Scalar dispatch charges and collects before an instruction runs,
so the accepted quantization is observable both *inside* a folded survivor — a metamethod invoked by the
commit-late `OP x, x, s` survivor, or a `Call` survivor, seeing the group's trailing boundaries already fired
— and from any guest code running *after* the group whose reachability the survivor's own execution would
have changed (a metamethod that nils the last strong reference to a weak-table value fires before the
optimized program's mid-group boundary would, while the unoptimized program's earlier, exact boundary sees
the old value); either way it is at most one group earlier than the unoptimized program would collect. The
fix does not widen this (the `k = 1` case quantized the same way before), and
neither the conformance corpus nor `tests/o2_schedule.rs` (numeric operands only) exercises it, so a
fixture with a metamethod-invoking survivor observing a weak table at a boundary belongs before any default
flip. Schedule equivalence is pinned by tests: identical minimum fuel per family, identical outcomes at
every fuel prefix, an identical logical `os.clock` on the plain-mode tape, identical results, collection
counts, and next boundaries across deterministic-GC interval sweeps
(`deterministic_collection_cadence_survives_interval_sweeps`), the lattice invariant on the six tracked
workloads under every family across two consecutive calls on both profiles
(`deterministic_collection_count_tracks_the_unit_lattice_across_calls`) plus a bounded
`ReproducibleStandard` heap across four calls
(`reproducible_standard_heap_stays_bounded_across_calls_for_unit_carrying_programs`; all three in
`crates/rustia-compiler/tests/o2_schedule.rs`), and the
straddling charge itself on a hand-built eight-unit schedule at every instruction edge, on both executors,
with and without a stopped collector (`crates/rustia-vm/src/vm.rs`
`unit_charged_steps_collect_once_per_crossed_boundary`,
`stopped_collector_still_advances_the_lattice_over_unit_charged_steps`).

The closure-call fast path synchronizes a mutated capture cell back into any
caller register aliases. The loader now caches a conservative bytecode proof:
any `GetLocalCell`, `SetLocalCell` or `LoadFunction` makes a function potentially
cell-producing, including unreachable or malformed instructions. Functions
without those operations cannot attach a local capture cell, so their leaf calls
skip the register-alias scan. The proof does not trust `captured_registers`
hints, and unknown/unretained frames retain the scan. Resume and stackless
self-calls retain the same immutable function proof. Fuel, collection guards,
mutation and logical instruction accounting are unchanged. The proof travels with
the existing caller-site cache lookup on eligible scalar calls; frames without
such calls do not perform an extra program/prototype lookup. This is a candidate
for the measured PR #74 closure regression; remote acceptance is still required.

Frame entry synchronizes saved register cells before applying child results and
rebuilding stackless roots. `resumed_callers_observe_mutated_capture_cells`
covers mutation while suspended, collection, and child mutation before return,
with and without fuel metering.

The PR #74 math follow-up passes validated unary arguments directly to the shared
math implementation, avoiding a temporary copy. Argument validation and numeric
operations are unchanged. The separate table-probe and fresh-frame scan changes
were reverted after failed remote acceptance; their tests and evidence remain.
The isolated math change still requires remote acceptance.

The full-corpus conformance gate (`rustia-conformance/tests/o2_conformance.rs`, threaded through
`run_repository_with_experimental_o2`) then split the families by an empirical reachability boundary:

- **Corpus-green (`ExperimentalO2Options::reachability_exact()`):** constant-branch folding, commit-late
  compound/self-update assignment, provably-nil tail folds, and slot tightening pass all 712 fixtures
  individually and together. These rewrites never extend or shrink object reachability at any deterministic
  collection boundary: bypassed temporaries are provably nil beforehand and every produced value stays rooted
  through its destination register. On the five tracked workloads this track removes 0.1-11.0% of executed
  instructions (dominated by the plain self-update accumulator shape, the C2.2 six-operation receipt).
- **Blocked on C2.4 (`elide_dead_cleanup_clears`):** deleting a standalone dead-value clear keeps the stale
  value register-rooted through later collection boundaries; the upstream
  `weak_table_key_value_root_liveness` fixture observes exactly one extra surviving weak entry. This family
  carries most of the measured dynamic reduction (24.7-35.3% per workload, 30.1-38.8% with every family) and
  stays hard-blocked until C2.4's frame root scanning masks registers the unoptimized program would already
  have cleared; a self-documenting ignored conformance test pins the divergence and flips into a passing gate
  when that lands.

**Enablement is an executor trade only where the rewrite lands in a hot prototype (measured 2026-08-26,
gate made per prototype 2026-09-15).** A prototype whose own slice of the logical-unit schedule is
non-identity loses its HotIR loops and its leaf-call plan, because both charge a fixed count per matched
PC that is exact only under one unit per instruction: `FunctionInlineCaches::new` (reached from
`LoadedProgram::new`, the load site) gates HotIR on the prototype that contains the `ForLoop`, and
`recognize_leaf_call_plan` gates a plan on the callee it replays (`ProgramLogicalUnits::function_is_identity`
/ `Program::logical_units_are_identity_for`). A rewrite in a different prototype deselects nothing; the
caller's schedule does not matter to a leaf plan because the `Call` site charges its own units on top of
the plan's fixed charge (`try_closure_call_scalar` widens its admitted window by the same units — before
2026-09-15 it charged and admitted `+1`, exact only for a caller with a unit-one `Call`; the fast path is
refused under fuel, so fuel never saw it). Until 2026-09-15 the gate was program-wide — `logical_units()
.is_some()` skipped HotIR and every leaf plan the moment any function carried a schedule — which is what
the 2026-09-13 host measurement below priced. `Program::with_logical_units` still drops a fully-identity
schedule, so a program the pass did not actually rewrite keeps everything. The default-off
`executor-diagnostics` feature and `crates/rustia-compiler/tests/executor_ownership.rs` report which
executor owns each workload and print which prototypes carry a non-identity slice. Leaf-call plans are
counted on both of their paths — the `Call` opcode's inline cache (`closure_call_fast_entries`) and the
dynamic `call_value` path that generic-for iterator steps, host `call_function`, `pcall`/`xpcall`, and
coroutine starts take (`leaf_call_value_entries`, recorded on the callee's program at
`try_leaf_bytecode_handle`'s success return; added 2026-09-15 after the finding below); the inline-cache
fill counters are now recorded under the feature too, not only under `cfg(test)`. At `n = 32`, with the
reachability-exact track (per-prototype gate; the pre-fix column is kept where it differed):

| workload | default | with `reachability_exact()` |
| --- | --- | --- |
| `dense_tables` | 1.03% tape, 16,298 HotIR iterations, 38 receipts | 99.75% tape, 0 HotIR, 8,187 receipts — unchanged by the per-prototype gate, because `f` is the only prototype and commit-late fires on `s = s + t[i]` *inside* its sum loop, so `f`'s own slice is non-identity and both of its loops (the untouched fill loop too) leave HotIR (99.76% and 8,191 before the 2026-09-15 lattice fix: its 22 boundary instructions now cut the tape window and run scalar, root refreshes 68 -> 86, the default side's count) |
| `sparse_tables` | 92.39% tape, 8,190 tape guard exits (**the only workload with any**) | unchanged coverage, 8,190 guard exits (receipts 2,080 -> 2,048; `f` carries the only non-identity slice) |
| `closures_upvalues` | 68.67% tape, 32 closure fast entries | 68.67% tape, 32 closure fast entries, identical census (77.67% tape and 0 fast entries under the program-wide gate; the counter closure `fn#1` is identity, `f` = `fn#2` carries the rewrite) |
| `generic_iteration` | 38.02% tape, 2,208 receipts, 1,055 `call_value` leaf replays of the `__iter` closure (33 per unit less one per collection boundary that lands in the plan window), 31 HotIR iterations from `make(32)` | 38.02% tape, 1,184 receipts, 1,055 leaf replays, 31 HotIR iterations (67.24% tape, 1,215 receipts, 0 replays, 0 HotIR under the program-wide gate; `f` = `fn#3` carries the only non-identity slice) |
| `stdlib_math_table` | 82.38% tape | identical counts (1,771 executed, 82.38% tape, 133 root refreshes) although the rewrite fires — commit-late at two sites plus one dead-tail fold, static 110 -> 102, schedule attached on `f` — because the workload has no HotIR loop or leaf closure to lose |
| `array_mutation` | 0.96% tape, 8,399 HotIR iterations | unchanged (no rewrite fired, no schedule attached) |

**Four of the six tracked workloads now isolate the rewrite from the executor**
(`per_prototype_gate_keeps_every_untouched_fast_path` pins it by counting: identical charged instructions
and checksums, every fast path in an unrewritten prototype served on both sides — `n` closure fast entries
and `33n` iterator replays less at most one per collection boundary, 31 HotIR iterations in `make` —
and `artificial_schedule_outside_the_hot_prototype_changes_nothing` attaches a hand-built schedule to the
all-off bytecode: a unit on main changes nothing but the one charged unit, a unit on the counter closure's
`Add` or on the first pc of `f`'s fill loop drops exactly that prototype's fast path, and every variant
charges what the scalar dispatcher charges). `dense_tables` remains the exception: a `dense_tables` gate
measures the tape replacing HotIR at least as much as it measures the rewrite, because the rewrite lands
inside the hot prototype. A per-loop gate would restore only its fill loop; restoring both needs HotIR to
charge the loop's actual units (sum over `[body_start..=backedge]` per continuing iteration,
`units[backedge]` on exit), which removes the HotIR gate entirely but re-enables a lane whose cost against
the tape under O2 is unmeasured — deferred until item 4 below says whether it is wanted. Any
`dense_tables` enablement number is an explicitly labelled combined executor-swap-plus-rewrite result.

`sparse_tables` is the only tracked workload whose operations miss the tape's dense-array guard, and the
test asserts that rather than only printing it. Its inner loop issues four index operations per iteration
(two writes, two reads) over 64 iterations per unit, all against hash-part keys, so exits track
`4 x 64 x n`: 2,048 at `n = 8`, 4,096 at `n = 16`, 8,190 at `n = 32`, 16,381 at `n = 64`. The small
shortfall at larger `n` is one exit per deterministic-collection boundary that lands on such an operation,
because a boundary instruction runs scalar and never enters the tape. Set `RUSTIA_EXECUTOR_CENSUS_N` to
re-check the scaling. This is a count of side exits with no cost attached to it; whether they cost
measurable time is what the re-frozen 201-repetition `sparse_tables` gate exists to answer.

Charged `executed_instructions` are identical in every row, which is the logical-receipt contract
working: the reduction the preview reports is in dispatched work, not in the charged schedule. The sign
of the trade can still differ per workload — `dense_tables` swaps HotIR for near-total tape coverage
while the others keep their executor — so enablement needs a per-workload controlled gate, not one gate
generalized across the cohort.

#### Measured on the owned host: the default is refused, and why (2026-09-13)

`reachability_exact()` was made the `CompileOptions` default on a throwaway candidate (`17e64af`: main `16b3c26`
plus that one change) and measured through the acceptance lane on `ci-bench`, eight legs a side at the lane's
fixed work (`dense_tables` n=2,786, `sparse_tables` n=1,449, `closures_upvalues` n=759,888, `generic_iteration`
n=9,969, `stdlib_math_table` n=107,256), checksums identical on both sides of every workload:

| workload | candidate vs main | interval | reading |
| --- | --- | --- | --- |
| `dense_tables` | -13.58% | -14.04%..-13.19% | faster |
| `stdlib_math_table` | -9.63% | -10.70%..-7.24% | faster |
| `sparse_tables` | +0.83% | -0.31%..+1.60% | no change measured |
| `generic_iteration` | +114.41% | +111.89%..+117.66% | slower |
| `closures_upvalues` | +188.00% | +183.47%..+194.13% | slower |

Both regressions are uniform across all sixteen legs (`closures_upvalues` 183-188 -> 527-554 ns/unit on every
pass), so they are the compiler's output, not the host. The mechanism was established by counting on the
`executor-diagnostics` census — no developer-host timing — at the fixed n of `dedicated-history-7a9e87d.json`
(per-unit deltas were linear in n), with each of the four families ablated alone and two counterfactuals: the
rewritten bytecode with its schedule stripped restores every fast path, and the unrewritten bytecode with an
artificial non-identity schedule reproduces the whole regression profile. Three independent attempts to refute
it failed.

- **Responsible family: `commit_late_compound_locals`, alone.** It fires once per regressed workload, on the
  `acc = acc + rhs` self-update, deleting three instructions per site; the other three families are
  byte-identical to all-off on these workloads. The rewritten shape itself is benign: it removes 3 (closures)
  and 96 (generic) dispatched operations per unit and would be a small win on its own.
- **The loss was the program-wide gate (fixed 2026-09-15, per prototype; see the enablement paragraph
  above).** `LoadedProgram::new` (the load site) and `recognize_leaf_call_plan` skipped HotIR and every
  leaf-call plan for the whole program once any function carried a non-identity schedule. The hot callees —
  the `UpvalueCounter` closure in `closures_upvalues` and the `ArrayIterator` `__iter` closure in
  `generic_iteration` — are byte-identical on both sides and still match their recognisers; they lost their
  plans only because a *different* function was rewritten. Every such call then built a full bytecode
  frame. At n=742,545, closure fast entries went 741,951 -> 0 and GC root refreshes 2,681 -> 742,558 per
  timed call; at n=10,364, `Len`-triggered refreshes went 648 -> 342,012 (33 lost iterator replays per
  unit), tape entries rose by 131 per unit, and Rust allocations went 1.80M -> 4.82M per call. With the gate
  keyed on the prototype's own slice, both workloads keep every fast path under `reachability_exact()`
  with an identical census to all-off (table above), and the fix surfaced a latent charge error on the
  `Call` fast path (it added 1 for the `Call` instead of the `Call`'s units — invisible on these workloads,
  whose rewritten `Call` pcs carry one unit — now pinned by `leaf_fast_path_charges_the_callers_call_units`
  with an artificial three-unit `Call`).
- **Two rows of the table above were wrong** (corrected 2026-09-15), because `try_leaf_bytecode_handle` — the
  iterator-path leaf replay — recorded no census counter, so `executor_ownership.rs` could not see it.
  `generic_iteration`'s executor was not "effectively constant" across the sides: it loses 33 `ArrayIterator`
  replays per unit. And `stdlib_math_table` was not "unchanged (no rewrite fired)": commit-late fires at two
  sites plus one dead-tail fold, static 110 -> 102, schedule attached. The enablement-gate argument built on
  the first of those rows does not hold. The counter now exists (`ExecutorCensus::leaf_call_value_entries`,
  recorded on the callee's program at the replay's success return and nowhere on its fallbacks) and
  `report_executor_ownership` pins it on both sides: 33 replays per unit less at most one per
  deterministic-collection boundary (1,055 at `n = 32`; the boundary instruction runs scalar, and each
  refused call shows up as a `Len`-triggered root refresh from the full frame it built), and 0 on the
  `Call`-opcode path; `closures_upvalues` pins the mirror image (`n` fast entries, 0 replays) on both sides.
  When the counter landed it pinned 0 replays under `reachability_exact()` — the program-wide gate — and
  the pin flipped to both-sides-serve with the per-prototype gate the same day.
  `crates/rustia-vm/src/vm.rs` unit tests prove the counter on host `call_function` calls: three
  `ArrayIterator` calls count 3 with identical results and charged instructions against the interpreter
  (`leaf_call_value_path_records_census_and_matches_interpreter`), and 0 when a fuel budget makes the path
  fall back (`leaf_call_value_path_counts_nothing_when_policy_falls_back`).
- **The wins are not pure rewrite wins either.** `dense_tables` sums the rewrite (dispatched operations
  -11.0%), the HotIR -> tape swap (1,146,459 HotIR iterations -> 0, 576,255 fused receipts at n=2,251), and the
  schedule effect described next; `stdlib_math_table` sums the rewrite (-5.67% dispatched) and that effect;
  `sparse_tables`, where the rewrite removes 0.09%, is the one workload that did not move. Counting cannot
  apportion the wins; only the measurements listed below can.

**A determinism bug in the schedule (found 2026-09-13, fixed 2026-09-15).** The scalar dispatcher advances
`executed_instructions` by `1 + extra_units` for a folded instruction; `maybe_collect` tested the
deterministic-collection lattice for exact equality, and the lattice advanced only on a hit. A multi-unit
instruction that stepped over the boundary therefore disabled the boundary collection for the rest of the
VM's life: `weak_collection_due_within` wrapped, the tape's admitted window became unbounded, and only
`reset_decision` re-armed it. Under the candidate the timed call performed almost no boundary collections
(`dense_tables` 1,568 -> 3 per call at n=2,251; `stdlib_math_table` 616 -> 25 then 0; `closures_upvalues`
2,079 -> 4), `ReproducibleStandard` heaps grew unreclaimed across calls (`generic_iteration` 3.2 -> 6.6 -> 10.0
MB after three calls on one VM), the measured wins include skipped collections, and the regressions understate
the executor loss. Two programs with identical charged `executed_instructions` collected on different
schedules, which is the invariant the logical receipts exist to hold; `tests/o2_schedule.rs` could not see it
because it compared results after an explicit `collectgarbage()`. The fix is the loop described under C2.2
above: `Vm::maybe_collect_with_roots` collects once per crossed boundary and advances the lattice once per
crossed boundary (`while executed >= next`, saturating at `u64::MAX` instead of wrapping), on both profiles,
so `deterministic_collection_count == floor(executed / interval)` and `next == (floor(executed / interval) +
1) * interval` hold after every charge; every fast tier admits work only while `weak_collection_due_within`
is false, so none of them needs the loop, and a `debug_assert!` pins `executed < next` at every admission
check. `tests/o2_schedule.rs` now observes the lattice directly through `Vm::executed_instructions`,
`Vm::deterministic_collection_count`, and the hidden `Vm::next_deterministic_collection_instruction`: the
interval sweep compares count and next boundary on a `DeterministicSim` leg
(`deterministic_collection_cadence_survives_interval_sweeps`), the six tracked workloads are checked under
all-off, each family alone, and `reachability_exact()` across two consecutive calls on one VM at intervals
{1, 7, 13, 97, 997, 10,000} (`DeterministicSim` in full, `ReproducibleStandard` at 1, 97, 10,000), with
every variant charging the all-off clock and the corpus-green variants returning the all-off results
(`deterministic_collection_count_tracks_the_unit_lattice_across_calls`), and a `ReproducibleStandard` heap
bound (each variant within 16 KiB of its own first call and of all-off after each of four calls at n=128,
intervals 97 and 10,000; `reproducible_standard_heap_stays_bounded_across_calls_for_unit_carrying_programs`)
pins the memory the skipped collections had left behind. The numbers above remain the record of the bugged
candidate; pricing the collections it skipped is item 4's `main + (2)` host measurement.

**A second determinism bug: fuel exhaustion mid-group (found and fixed 2026-09-15).** The scalar dispatcher
charged a folded instruction's whole restored cost -- its own fuel plus every unit it absorbed -- in one
`charge_fuel` call, before advancing `executed_instructions` or calling `maybe_collect`. When that charge
failed with `FuelExhausted`, the `?` discarded the group's progress entirely: `executed_instructions` never
advanced for it, whereas the unoptimized program, charging its atomic instructions one at a time, would have
advanced by however many of them fit in the remaining fuel before failing on the one that didn't. Two programs
with an identical schedule for a completed run could therefore leave the VM's `executed_instructions` and
deterministic-collection lattice at different points after a fuel-limited call exhausts inside a fused group --
the same invariant the lattice bug above violates, on a different trigger. `tests/o2_schedule.rs`'s
`every_fuel_prefix_matches_exactly` could not see it: it only compares the guest-visible (result-or-
`FuelExhausted`) outcome, which the restored total-fuel-need invariant
(`minimum_fuel_is_identical_for_every_family`) already guarantees agrees regardless of this bug.
Reachability: not exposed through any public embedder surface today -- `RustiaBuilder`/`Rustia` carry no
`ExperimentalO2Options` knob at all, so O2 is reachable only by code that drives `compile_with_options` and
`rustia_vm::Vm` directly -- and every fast tier (tape, HotIR, leaf-call plans) already refuses admission
whenever fuel is bounded (`self.settings.initial_fuel.is_some() || fuel.state.fuel.is_some()`), so fuel-limited
execution always falls back to this exact scalar-dispatch charge site regardless of which family is enabled;
no separate fix was needed there. The one place in the workspace that already combines O2 with a bounded fuel
budget (`crates/rustia/tests/error_recovery.rs`) uses a budget generous enough that it never actually exhausts.
Fixed by charging one folded unit at a time -- each costs exactly one fuel, the invariant the restored schedule
already relies on -- with its own `executed_instructions` advance and `maybe_collect` call, stopping at the
exact unit a `FuelExhausted` budget cannot afford; real dispatch still only runs once every unit in the group
is charged, so the group's own semantics (which can invoke a metamethod for `commit_late_compound_locals`'s
folded `Add`/`Sub`/`Concat`/... op) stay all-or-nothing, matching what the unoptimized program's own failing
instruction would do.
`crates/rustia-compiler/tests/o2_schedule.rs::fuel_exhaustion_mid_group_leaves_the_same_schedule_as_the_unoptimized_program`
reproduces the divergence directly (confirmed failing before the fix -- `executed_instructions` 10 vs 9 at fuel
10 on `local total = 0; for i = 1, 5 do total += i end`) and sweeps every fuel level from 0 to `minimum_fuel +
2`, comparing `executed_instructions` and the deterministic-collection lattice, not just the outcome.

**Memory-limited execution uses scalar quota boundaries (found 2026-09-15; resolved 2026-10-18).**
HotIR and the executable tape do not call `enforce_memory_limit` inside their batched execution. Their
array-write lanes could therefore grow a table until the next deterministic collection boundary before
the scalar dispatcher detected the quota breach. An ordinary unmetered
`RustiaBuilder::standard().memory_limit_bytes(n).build()` was affected. Typed invocations already
supplied fuel, which excluded both tiers.

Memory-limited VMs now reject HotIR, tape, and leaf-call admission, regardless of their fuel setting,
profile, or collection interval. Array growth runs through the scalar dispatcher's existing per-instruction quota
checks and collection retry. This preserves the documented instruction-boundary limit; it does not
promise that an instruction's transient allocation or allocator-reserved capacity stays below that
limit. Although the leaf-call plans do not allocate, they can mutate an upvalue before checking growth
from the preceding instruction. They therefore use the same scalar admission policy to preserve quota
error precedence and prevent that mutation after a quota breach. No periodic checks were
added inside a fast executor. Restoring those executors under a quota requires equivalent
instruction-boundary enforcement and remote `ci-bench` evidence before making performance claims.

`crates/rustia/tests/hot_path_memory_limit.rs` now compares unmetered and fuel-forced scalar execution for
numeric and non-numeric array assignments across all four profiles and collection intervals
{1, 8, 128, 1,000, 10,000}. Both must return `OutOfMemory` with identical live and peak memory and
single-instruction quota precision. Hardened-leaf invocation coverage retains the same tight boundary.
Memory tracking does not force scalar dispatch. An instrumentation-free VM regression also executes a
program known to enter both HotIR and tape without a quota, then verifies that a memory-limited VM
returns the same result and charged instruction count with neither tier entered. These are correctness
checks; they make no performance claim. A near-limit `NewArray` immediately followed by a counter
leaf call also verifies that `OutOfMemory` precedes the leaf's upvalue mutation, matching the explicitly
scalar reference's instruction count.

**What follows.** The default stays off, and per-workload enabling was not the answer while the gate was per
program: any program with a leaf-shaped closure or a HotIR loop regressed the moment one deletion fired
anywhere in it (`array_mutation`, 8,399 HotIR iterations per call, was next in line). The counts pointed at:

1. **Done (2026-09-15).** HotIR and leaf-call plans are gated per prototype: skipped only where that
   prototype's own slice of the schedule is non-identity (HotIR on the prototype containing the loop, a leaf
   plan on the callee it replays; the `Call` site charges its own units on top of the plan, which also fixed
   the fast path's `+1` call charge). Counting acceptance, met on the tracked workloads
   (`executor_ownership.rs`, `n = 32`): closure fast entries back to `n` (32), `Len`-triggered root
   refreshes back to the default side's count (one, pinned as at most one per collection boundary plus
   one), `generic_iteration`'s 31 HotIR iterations and 1,055 iterator replays restored, charged
   `executed_instructions`, root refreshes, and checksums identical on both sides of every tracked
   workload. The one caveat is `dense_tables`: its rewrite lands inside `f`'s own HotIR loop, so `f`'s
   slice is non-identity and its HotIR stays off under `reachability_exact()` (0 iterations, tape 99.75%)
   — restoring it needs unit-aware HotIR charging, not a gate, and is deferred to the measurement in item
   4. VM unit tests pin the mechanism on hand-built schedules (`hot_ir_gate_is_per_prototype`,
   `leaf_plan_is_gated_on_the_callee_slice_only`, `leaf_fast_path_charges_the_callers_call_units`: results,
   charged instructions, collection counts, and next boundaries identical to the scalar path at every
   interval 1..=23), and `executor_ownership.rs` pins it on the tracked workloads
   (`per_prototype_gate_keeps_every_untouched_fast_path`,
   `artificial_schedule_outside_the_hot_prototype_changes_nothing`).
2. **Done (2026-09-15).** The boundary test is robust to unit-charged steps: the lattice advances once per
   crossed boundary, collecting at each, and the counting tests pin
   `deterministic_collection_count == floor(executed / interval)` for every tracked workload under every
   family across an interval sweep and across consecutive calls on one VM
   (`crates/rustia-compiler/tests/o2_schedule.rs`
   `deterministic_collection_count_tracks_the_unit_lattice_across_calls`,
   `deterministic_collection_cadence_survives_interval_sweeps`), plus the bounded-heap assertion under
   `ReproducibleStandard` (`reproducible_standard_heap_stays_bounded_across_calls_for_unit_carrying_programs`)
   and the straddling charge itself on a hand-built schedule (`crates/rustia-vm/src/vm.rs`
   `unit_charged_steps_collect_once_per_crossed_boundary`,
   `stopped_collector_still_advances_the_lattice_over_unit_charged_steps`). The tape's window cut is
   unchanged.
3. **Done (2026-09-15).** A census counter at `try_leaf_bytecode_handle`'s success
   (`leaf_call_value_entries`), so iterator-path replays are visible to `executor_ownership.rs` and to the
   table above; `report_executor_ownership` pins it on both sides of every tracked workload and the VM
   unit tests (`leaf_call_value_path_records_census_and_matches_interpreter`,
   `leaf_call_value_path_counts_nothing_when_policy_falls_back`) pin the counter itself, as described in
   the bullet above.
4. **Pending (owned host).** No timing of the fixed tree exists; the 2026-09-13 table above is the only
   owned-host number for this track, and it prices the program-wide gate plus the lattice bug, not the
   tree that (1)-(3) produced. Measure on `ci-bench`, eight legs a side: this tree (main plus (1)-(3)) with
   the default off against main — default-compiled bytecode and its schedule are byte-identical, so the
   prediction is within noise, and that leg tests the prediction rather than assumes it; the same tree with
   `reachability_exact()` on against that (predicted from the counts, not measured: `closures_upvalues` and
   `generic_iteration` within a few percent of baseline, `sparse_tables` flat, the stdlib win shrinking to
   its rewrite share, `dense_tables` still summing the rewrite and the HotIR -> tape swap, which that leg
   prices); and main plus (2) alone against main, which prices the skipped collections. Only when the
   second of those shows no workload above its threshold does this section flip the default; whether
   `dense_tables` wants unit-aware HotIR is decided by the same numbers.

Default compilation remains byte-identical and schedule-identical; enabling any corpus-green family by
default is now purely a measured performance decision behind the standard controlled A/B gates.

### C2.4 — register arena and iterative frames

- Add safe reusable value/cell pages owned by the VM.
- Convert ordinary calls, returns, tail calls, protected unwinds, yields/resumes, and coroutine snapshots one
  path at a time to compact index-based frames.
- Scan active windows directly for GC roots. Differentially exercise recursion, `pcall`/`xpcall`, coroutines,
  open captures, host re-entry, errors, quotas, and reset.
- Mask liveness-dead registers during frame root scans using the C2.3 analysis, so eliding standalone
  dead-value cleanup clears (`elide_dead_cleanup_clears`, which carries most of the C2.3 dynamic reduction)
  stops changing weak-table observability; the pinned `weak_table_key_value_root_liveness` divergence is the
  acceptance test.

**Measured size of the root-scan prize (2026-08-26).** `Vm::update_current_gc_roots` clears the current
frame's root vector and copies the whole register file, capture cells, varargs, and pending return
prefixes into it, and it runs at every instruction `instruction_may_invoke_guest` reports — which
includes `GetIndex`/`SetIndex` unconditionally, plus every call, global access, `Concat`, and `Len`. The
`executor-diagnostics` census counts that traffic per workload at `n = 32`:

| workload | executed | root refreshes | values copied | copies per executed instruction |
| --- | --- | --- | --- | --- |
| `sparse_tables` | 111,731 | 12,297 | 196,752 | 1.76 |
| `generic_iteration` | 36,955 | 2,509 | 37,193 | 1.01 |
| `stdlib_math_table` | 1,771 | 133 | 1,699 | 0.96 |
| `dense_tables` | 222,933 | 86 | 1,290 | 0.006 |
| `array_mutation` | 166,786 | 16 | 240 | 0.001 |

**Ceiling on the phase, computed before building it (local screen, 2026-08-26).** Counts are not time, so
the prize was bounded before committing to the rewrite. Both arms disable deterministic collection
(`deterministic_gc_interval = u64::MAX`), which lets a build whose `update_current_gc_roots` body is a bare
`return` stay checksum-identical to one that refreshes; the delta is then the whole cost of maintaining
copied frame roots. Release build, `n = 32`, 40 repetitions, medians:

| workload | refresh intact | refresh removed | delta | copies per instruction |
| --- | --- | --- | --- | --- |
| `generic_iteration` | 139,917 ns | 117,875 ns | -15.8% | 1.01 |
| `sparse_tables` | 1,050,875 ns | 915,000 ns | -12.9% | 1.76 |
| `stdlib_math_table` | 15,875 ns | 13,709 ns | -13.6% | 0.96 |
| `dense_tables` | 490,667 ns | 482,458 ns | -1.7% | 0.006 |

The ordering tracks the copy counts, and the HotIR-owned workload is ~nothing, which is what makes the
screen credible as a measurement of the intended quantity rather than of noise.

At `n = 32` the smaller workloads' effects are close to the run-to-run variance, so the screen was repeated
with each workload's `n` raised until baseline runtime is milliseconds rather than microseconds (60
repetitions, medians):

| workload | n | refresh intact | refresh removed | delta (median) | delta (min) |
| --- | --- | --- | --- | --- | --- |
| `generic_iteration` | 1,024 | 4,453,667 ns | 3,481,458 ns | -21.8% | -20.8% |
| `stdlib_math_table` | 8,192 | 4,221,750 ns | 3,489,083 ns | -17.4% | -16.2% |
| `sparse_tables` | 256 | 8,348,375 ns | 7,290,833 ns | -12.7% | -15.2% |
| `dense_tables` | 256 | 3,994,125 ns | 3,829,375 ns | -4.1% | -0.2% |

With multi-millisecond baselines the three tape/scalar-owned effects are far larger than the variance, and
`stdlib_math_table` in particular resolves rather than staying inside the noise as it did at `n = 32`.

`dense_tables` is ~zero, and the median/min split says so rather than merely failing to decide. Min is the
least-contaminated sample — the run where least interference landed — and it moves far less with machine
load than the median does. A real 4% effect on 222,933 executed instructions would appear in both; a median
that reports 4.1% while the min reports 0.2% is the shape of interference the min escaped. That agrees with
the mechanism, since `dense_tables` is HotIR-owned at 0.006 copies per executed instruction and has almost
nothing for refresh removal to bite on. Two independent lines pointing at ~zero is a stronger statement
than an undecided one. **Corollary for any dev-machine screen here: when min and median disagree by
multiples, prefer min.** The cloud protocols deliberately use pooled medians for distributional reasons;
that choice does not transfer to an unqualified developer machine.

The benchmark binary carries none of this instrumentation, verified on the artifact rather than argued from
the `cfg` attributes: `cargo tree -p script-bench-harness -e features` contains no `executor-diagnostics`
entry, and `nm target/release/script-bench` matches zero symbols for `note_root_refresh_trigger`,
`executor_census`, `gc_root_refresh_histogram`, `record_tape_execution`, or `record_hot_ir_entry`.

**What the ceiling is a ceiling on.** Disabling collection removes collection work from both arms equally,
so it does not bias the comparison, but production pays refresh cost *and* collection cost and C2.4 changes
the split between them rather than only deleting the first. These figures bound the refresh half.

Five caveats, all load-bearing. This is a **developer-machine screen and must never enter benchmark
history**; the Linux protocols remain the timing authority. Repeated runs of the same build varied by
about 7.5%, so read these as "order of 10-18% on the tape/scalar-owned workloads", not as point estimates,
and treat the dense figure as indistinguishable from zero. Disabling collection changes tape admission and
removes collection work, so production behaviour differs. The stripped build is unsound in general and
stays correct only because collection never runs. Most importantly this is an **upper bound**: C2.4
replaces per-instruction copying with a window scan at collection time, and that scan is not free, so the
realised win is strictly less than the table shows.

**Narrowing the predicate does not reach this traffic — only a window scan does (tried and reverted,
2026-08-26).** The obvious cheap alternative is to make `instruction_may_invoke_guest` heap-aware, since a
table read whose receiver carries no metatable can neither re-enter guest code nor allocate. Implemented
against `GetIndex`/`GetTableKs`, it changed **no workload's refresh count at all**. Two reasons, and both
say the same thing:

A null result alone could not tell "no table read reaches the predicate" apart from "the predicate is not
consulted where it was assumed to be", so the census also records which opcode triggered each refresh and
whether it came from the tape's exit path. At `n = 32`, defaults:

| workload | refresh triggers (`tape_exit` counted separately) |
| --- | --- |
| `sparse_tables` | tape_exit 8,190; `GetIndex` 4,096, `SetIndex` 4,096 (those same exits), `Concat` 4,096, misc 9 |
| `generic_iteration` | `GenericCallLoop` 1,184, `GeneralizedIterLoop` 1,024, `GetGlobal` 65, `CallMulti` 65, `GenericCallPrep` 64, `GetTableKs` 64, `GeneralizedIterPrep` 32, `SetTableKs` 8, misc 3 |
| `stdlib_math_table` | `FastCall` 96, `CallExpand` 32, `GetImport` 5 |
| `dense_tables` | `GetImport` 32, `FastCall` 32, misc 22 |
| `closures_upvalues` | `CallMulti` 8 |
| `array_mutation` | misc 16 |

That resolves it. The predicate *is* live on the scalar path — `generic_iteration` consults it for 64
`GetTableKs` — and the refinement correctly declined those, because that workload's receivers are `Coll`
instances carrying a metatable by construction (`Coll.__index = Coll`), so `__index` genuinely may be
consulted. Everything else is either a call, an allocation, a global access, or a concatenation that
genuinely can re-enter guest code, or it arrives through the tape's unconditional exit path where the
predicate is never asked.

So no refresh in the tracked set is an over-approximation the refinement could have removed. The traffic is
not something that can be tightened away; it is the cost of the frame owning a copied root vector at all. Removing it requires C2.4's active-window scan, which is why this was reverted
rather than kept as an unmeasured special case with its own GC-liveness reasoning.

The two HotIR-owned workloads pay almost nothing because HotIR does not refresh per iteration; the
tape/scalar-owned workloads copy roughly one to two `Value`s for every logical instruction they execute.
Scanning the active frame window directly replaces that copy with a borrow, which is why C2.4 is sized
against `sparse_tables` and `generic_iteration` rather than the dense workloads. This is a count of
copied values, not a timing claim.

This is the largest broad architectural step and the phase most likely to close the closure/call gap. It lands
only after C2.2 provides a stable executor boundary; mixing a frame rewrite with the old policy ladder would make
failures difficult to localize.

### C2.5 — direct tables and fixed-result calls (first slice implemented; the measurable target moved)

- Fast-path existing dense-slot mutation with incremental memory accounting and mutation-version updates.
- Add constant numeric slot forms and colocated PIC records.
- Extend exact-identity FASTCALL lanes and direct fixed-result writes without weakening monkey-patch fallback.
- Audit maintained memory counters against a debug full census after every mutation shape.

#### C2.5a — direct existing dense-array slot write

`VmCell::try_write_numeric_array_slot` mutates a proven existing, non-nil array slot in place. The guard is
exact and fully evaluated before any mutation: the handle resolves, `one_based_index - 1` is inside `array`,
and the value stored there is not nil. A present key bypasses `__newindex` in both layouts, so the lane needs
no metatable test, and a present array slot shadows any hash/node-part entry carrying the same numeric key —
`table_numeric_get` and `puc_getnum` both read the array first, and every array-growth site removes the
numeric hash key as it fills the slot. Overwriting the slot therefore cannot allocate, grow, shrink, rehash,
or change any array/hash length or capacity, which is exactly the `table_data_memory_usage` input set, so the
write skips `mutate_table_data`'s before/after census and its no-op `side_usage` adjustment and performs only
the single guest-visible `mutation_version` bump. Readonly still wins over the lane, storing nil is included
(it clears a present slot without moving the array boundary), and every other shape — missing handle, index
0, out-of-range index, existing nil slot — falls through to the unchanged generic path. The guard reuses the
handle lookup the generic path already needed, so a miss keeps its original single lookup.

The shadowing invariant is not an assertion in prose: a `debug_assert!` probes both hash parts on every
direct write, so the unit, conformance, and fuzz suites enforce it mechanically. Correctness coverage is a
Log/PUC differential matrix comparing shortcut-then-fallback against the general `table_set` path on values,
border length, iteration order, storage shape, mutation-version movement, and the memory census, plus
end-to-end Luau tests for both table layouts covering `__newindex` bypass on present keys, `__newindex`
dispatch on absent and present-nil keys, in-place nil clearing, `table.freeze` rejection, and the
`table.sort` mutation guard.

**Measured hit coverage: zero.** A default-off `table-write-diagnostics` feature censuses which lane serves
each numeric array-slot write, so no production or benchmark build carries hot-path instrumentation. The
census (`just check-array-slot-coverage`) reports that the direct lane is reached **0 times by all 17
runnable benchmark workloads**. Every numeric array-slot write in the suite is an append or a deferral:
`dense_tables` performs 16,384 writes per 64 units and all 16,384 grow the array, `stdlib_math_table`
performs 4,000 and all grow, `sparse_tables` defers all 8,191, and `memory_burst` performs 266,240 with 0
direct. Each tracked workload rebuilds its tables from `{}` and walks an ascending index, so it never
rewrites a slot it already filled. A tripwire test pins that fact and fails the day a tracked workload starts
reaching the lane, which is when the performance bet reopens.

Because the lane cannot be executed by a tracked workload, the exact DigitalOcean gates measured the only
thing the slice can cost one: the guard evaluated on every miss. `dense_tables` evaluates it 384,000 times
per timed repetition at `n=1500` and never takes it, so it is the correct regression gate. Three gates ran
on that fixed workload, each with 101 repetitions per leg, four counterbalanced ABBA/BAAB legs per side,
20,000 bootstrap iterations, and every leg matching checksum `number:193302491.00000000000000000`.

| gate | baseline | candidate | delta | conservative 95% interval | pooled interval |
|---|---|---|---:|---|---|
| lane checked before the layout dispatch | `92d30b8` | `066ce1f` | +0.6165% | -0.1571% to +2.2646% | +0.2334% to +1.1304% |
| lane admitted inside each branch | `92d30b8` | `2082436` | -0.4457% | -2.5751% to +1.5953% | -0.9741% to +0.1617% |
| same, on the C2.2 tape baseline | `b253e9f` | `0a31255` | -1.8842% | -18.1155% to +0.3730% | -3.5130% to -0.9452% |

The first gate's point estimate was consistent with a redundant load the first form introduced: it tested the
lane before the layout dispatch and re-read a slot the branch it fell through to was about to load anyway.
That measurement selected the final form, which admits the lane from inside each layout branch on the
`existing` value that branch already loads. The third gate re-measured the final form after the C2.2
executable tape landed on main, because the first two gates' commits were rebased away and their baseline
predates the tape; the tape still funnels dense-array writes through `try_write_numeric_array_slot`. Its wide
conservative interval reflects leg clustering rather than a large effect: two of its four baseline legs ran
about 20% slow (64,668.6 and 65,085.7 versus 53,613.0 and 54,014.6 ns/unit) while all four candidate legs
stayed within 53,071.2-54,269.0, so its -1.8842% point estimate should not be read as a win.

Those three gates all ran on workloads that never execute the lane, so all three are
`material_regression_excluded` with conservative intervals containing zero: they establish only that the
guard costs nothing where it never fires. The lane's actual value is measured separately below, on
`array_mutation`.

Fourteen ephemeral Droplets produced those three accepted gates. Eleven were rejected before or during
measurement - AMS3 on steal, AMS3 at a 36.342% within-order paired ratio spread against the 5% ceiling, NYC3
on steal, SFO3 at 5.098%, 6.689%, 11.403%, 27.537%, 17.578%, 12.997% and 8.591%, plus one accepted block
invalidated by steal at leg 7. This account offers only shared Basic plans, so region choice and re-drawing
were the available levers. Every Droplet and temporary key was confirmed deleted, and 225.2 billable minutes
cost about $0.31. The evidence for all three gates was measured on those rented Droplets and was removed on
2026-09-13; the samples remain in Git history. The first two cohorts name commits that the tape rebase removed
from the branch.

The measurable dense-table opportunity is array append and growth, where every one of `dense_tables`' numeric
array writes lands - not existing-slot mutation. Capacity prediction and append fast-pathing are a separate
slice with their own gate and must not be folded into this one.

#### C2.5b — direct array-append lane

`dense_tables` takes the C2.5b append lane on all 16,384 of its numeric
array-slot writes and reaches the generic `mutate_table_data` path zero times, so
unlike C2.5a this slice is measurable on a tracked workload without adding one.
The guard requires a Log-layout table, an existing nil/out-of-range slot, no
metatable, not readonly, a non-nil value, `one_based_index == array.len() + 1`
exactly, and an empty Log hash part. That last condition is what makes the
collapse sound: the generic path's resize + overwrite + `remove_hash_key`
sequence exists to absorb a numeric hash entry sharing the new index, and
`table_hash_lookup` returns `Vacant` before it ever reads the side index, so
empty `entries` is definitional proof that no such entry exists anywhere. The
whole sequence becomes one `Vec::push`. An append does grow the array, so unlike
C2.5a this cannot skip accounting, only make it incremental: live rises by
`size_of::<Value>()` and reserved by the capacity delta, with the hash part
untouched.

An adversarial pass attacked accounting drift (residual hash capacity, a hollow
side index, `table.clear`, `release_spare_capacity`, reallocation), semantic
divergence, guard holes and version bumping across a 480,000-operation
interleaved differential stress run, and found no defect.

The measurement is the first taken on the dedicated `ci-bench` host after
DigitalOcean was retired. Control `4e5eb70` is candidate `eaf3917` with only the
append-lane admission removed. On fixed `dense_tables` work at n=1500, 201
repetitions per leg and counterbalanced order, pooled medians were 39,898.435
versus 39,184.492 ns/unit: **-1.7894% with a conservative nominal 95% interval of
-2.1250% to -1.0770%**. The interval excludes zero, so this establishes a
speedup. All 8 legs matched checksum `number:193302491.00000000000000000`, and
every candidate leg median fell below every control leg median. Evidence and the
protocol deviations are under
`bench/history/evidence/eaf3917f7618c08852c5f03d81ebb2db0b6f6f6f/`.

#### Closing the workload gap

The suite had no in-place mutation workload at all, which is why C2.5's direct-table lane was unmeasurable.
`array_mutation` (`bench/workloads/luau/12_array_mutation.luau` plus Lua 5.4, JavaScript, Koto, and Rhai
ports) closes that gap: it builds a 256-element array once and then only rewrites filled slots, the shape of
particle, grid, and vector update loops. Roto carries no port because its `List` exposes `get`/`push`/`swap`
but no element assignment, and the hand-written WAT set exists only for the wasm pre-lock gate. The workload
is `default: false`, so the public five-workload cohort and its generated claim gate are unchanged until a
full cross-engine matrix admits it. Every port was checked against an independent reference implementation
and all five produce the same checksum, and the array stays non-degenerate (all 256 values remain distinct,
because `x -> (3x + it) mod 1000` is a bijection).

The C2.5a census now reports the mirror image of the tracked five: `array_mutation` serves 16,384 of its
16,640 numeric array-slot writes through the direct lane (98.46%), the remaining 256 being the one-time
build. A test pins that, alongside the tripwire pinning zero coverage on the tracked five.

#### The lane measured where it fires

With a workload that reaches the lane, the exact DigitalOcean gate compared control `7e6e71d` - candidate
`accca92` with only the lane's two admissions removed, so the two trees differ by nothing else - against
candidate `accca92` on fixed `array_mutation` work at `n=4000`. Pooled medians were 32,833.387 ns/unit for
the control and 30,125.987 for the candidate: **8.2459% lower candidate time, with a conservative nominal 95%
interval of -12.8420% to -3.1921%** (pooled -9.4445% to -7.2895%). That interval excludes both zero and the
predeclared +5% material-regression boundary, so this is accepted evidence of a speedup rather than a
neutral result. All 804 samples per side matched checksum `number:3996000.00000000000000000`.

The run also validated the repetition change. The 5% within-order paired ratio ceiling was tuned on
`generic_iteration`, and table workloads kept landing just over it at 101 repetitions per leg (5.098%,
5.490%, 6.689%). At 201 repetitions the same pool qualified on its first draw with 0.2412% and 0.7950%
within-order spreads. More repetitions shrink the leg-median standard error; the ceiling itself is unchanged.

Taken together the four gates say the lane is free where it never fires and worth about 8% where it does.
This is a per-workload result on one Linux protocol, and `array_mutation` is deliberately outside the public
five-workload cohort, so it is not a blanket speed claim and public benchmark history is unchanged.


## Measurement and stop rules

Every phase is an independently revertible commit with an exact candidate hash.

- Correctness validation may use local and CPU-busy self-hosted runners; no latency result from those machines is
  evidence.
- DigitalOcean A/B uses fixed work, identical toolchain/lockfile/harness inputs, counterbalanced order, retained
  raw samples, checksum equality, idle/steal qualification, and automatic droplet/SSH-key cleanup.
- A point estimate is not enough. Publish the conservative interval and the predeclared material-regression
  decision.
- Retain a slice that excludes the material regression threshold and has architectural value, but describe it as
  neutral unless the interval establishes a win. Revert a statistically clear slowdown unless it is a temporary,
  documented prerequisite with a near-term compound gate.
- Do not add more benchmark-shaped leaf recognizers after a miss. A failed C2 slice changes the executor/compiler
  design or closes the bet.

## Success definition

VM Core 2 is complete when the ordinary safe-Rust interpreter—not a JIT or workload-specific extension—uses the
block executor and frame arena on all supported native and wasm targets, passes the semantic/determinism gates,
and materially narrows the five-workload Luaur/reference-Luau gap on controlled Linux evidence. Parity is a
measurement outcome, not an architectural assertion.


### Numeric reads inside recognized array-iterator leaves

The `ArrayIterator` replay retains general table lookup. An attempted direct
materialized-slot read failed remote isolation and was reverted; see
[95](95-numeric-index-contract.md). Its forced-leaf versus scalar regression
remains: numeric array values, holes, exhaustion and fractional/negative/zero
keys across all four profiles, including a PUC51 hash-backed positive key.
The test verifies mutation, actual leaf entry and equal instruction counts.

### Scalar leaf-call metadata

Scalar leaf admission reads function kind and a borrowed capture slice from one
generation-checked function-arena lookup. After the existing plan, fuel,
collection and recursion checks, it copies only the selected pair before any
cache mutation. Capture selection failure is handled at the original guard-exit
point, preserving cache-fill/hit diagnostics. Captures are never cached across
calls, and malformed incomplete captures fall back without changing upvalues.
The polymorphic-handle/live-step and incomplete-capture regressions cover these
invariants. This removes a second handle lookup; performance is unaccepted until
remote comparison, as recorded in [95](95-numeric-index-contract.md).

### Reusing iterator function resolution

A custom iterator's function handle/kind is resolved after the existing
callability and state/control checks. Built-in next/inext handling consumes that
resolution and retains its existing depth/fuel/table checks. A valid non-built-in
function reuses the same resolution in the existing shared executors, with
iterator-specific entry glue after checking depth;
invalid handles and nonfunctions retain ordinary call-value fallback. Reuse does
not cross guest execution or heap mutation. Leaf replay, yielding, deferred calls,
caller-cell synchronization and result application remain in their existing paths.
The error-precedence regression covers invalid state/control registers, missing
handles, depth exhaustion and built-in table errors without capture mutation.
