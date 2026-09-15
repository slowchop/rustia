# VM Core 2 — interpreter parity without a JIT

> **Status (2026-08-24): C2.1 was remotely falsified; C2.2 Region-integrated private receipts were
> remotely accepted with 8.1651% lower ns/unit; the C2.2 mode-selected whole-function plain tape is
> implemented, correctness-validated, and remotely accepted with 4.0059% lower `generic_iteration`
> ns/unit against the receipt baseline.** This is the execution plan for the largest safe non-JIT performance programme. It refines the
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
per PC and are skipped for unit-carrying programs; deterministic collection boundaries stay on the original
count lattice but quantize to emitted-instruction edges inside a fused unit group. Schedule equivalence is
pinned by tests: identical minimum fuel per family, identical outcomes at every fuel prefix, an identical
logical `os.clock` on the plain-mode tape, and identical results across deterministic-GC interval sweeps.

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

**Enablement is an executor trade, not a free instruction win (measured 2026-08-26).** Attaching a
non-identity logical-unit schedule deselects HotIR *and* the leaf-call plans for that program
(`Vm::load_program` and `recognize_leaf_call_plan` both skip unit-carrying programs, because their
accounting assumes one logical unit per matched PC). `Program::with_logical_units` drops an identity
schedule, so a program the pass did not actually rewrite keeps both. The default-off
`executor-diagnostics` feature and `crates/rustia-compiler/tests/executor_ownership.rs` report which
executor owns each workload; at `n = 32`, with the reachability-exact track:

| workload | default | with `reachability_exact()` |
| --- | --- | --- |
| `dense_tables` | 1.03% tape, 16,298 HotIR iterations, 38 receipts | 99.76% tape, 0 HotIR, 8,191 receipts |
| `sparse_tables` | 92.39% tape, 8,190 tape guard exits (**the only workload with any**) | unchanged coverage, 8,190 guard exits |
| `closures_upvalues` | 68.67% tape, 32 closure fast entries | 77.67% tape, 0 closure fast entries |
| `generic_iteration` | 38.02% tape, 2,208 receipts | 67.24% tape, 1,215 receipts |
| `stdlib_math_table` | 82.38% tape | unchanged (no rewrite fired) |
| `array_mutation` | 0.96% tape, 8,399 HotIR iterations | unchanged (no rewrite fired) |

**The enablement gate runs on `generic_iteration`, not `dense_tables`.** A dense gate would measure the
tape replacing HotIR at least as much as it measures the O2 rewrites, because the executor changes under
the candidate. `generic_iteration` loses only 31 HotIR iterations out of 36,955 executed instructions, so
the executor is effectively constant across both sides while the rewrite effect is real (tape share
38.02% to 67.24%, receipts 2,208 to 1,215). Dense may still be measured, but only as an explicitly
labelled combined executor-swap-plus-rewrite result, never as the primary evidence for enabling a family
by default.

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
of the trade therefore differs per workload — `dense_tables` swaps HotIR for near-total tape coverage,
`closures_upvalues` loses its closure-call fast entries outright — so enablement needs a per-workload
controlled gate, not one gate generalized across the cohort.

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
