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
Evidence is under
`bench/history/evidence/84e732add4863150bd993f3354b274ab9c3ee41a/`.

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
Droplet deletion. Evidence is under
`bench/history/evidence/77e0d92871133277340b03b2eda119974babecda/`. Combined
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

### C2.3 — complete compiler O2 and cleanup liveness (analysis and machinery prepared)

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
- The optimization families themselves — constant-branch folding, commit-late/direct uncaptured compound
  assignment, target-directed dead Move/Clear tail removal, and register-file tightening — exist as
  default-disabled machinery with per-family differential tests, corpus-wide structural checks over the
  conformance fixtures, and a generated workload preview (`design/58-vm-core2-c23-o2-preview.json`, deferred
  with the machinery). They are deliberately not merged into main until the tape lands and logical receipts
  make the fuel/GC schedule optimization-independent; enabling any family before that is a declared
  compiler/fuel-schedule revision. Constant/RK operand emission is explicitly deferred: the census counts
  only 1-5 static candidates per tracked workload, far below the 33-69 removable clears, so spending new
  public opcode surface there is not yet measurement-justified; the analysis keeps counting candidates.

### C2.4 — register arena and iterative frames

- Add safe reusable value/cell pages owned by the VM.
- Convert ordinary calls, returns, tail calls, protected unwinds, yields/resumes, and coroutine snapshots one
  path at a time to compact index-based frames.
- Scan active windows directly for GC roots. Differentially exercise recursion, `pcall`/`xpcall`, coroutines,
  open captures, host re-entry, errors, quotas, and reset.

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

Verdict: retained as neutral. All three are `material_regression_excluded` and all three conservative
intervals contain zero, so none establishes a win or a loss, and the final form carries no nominal cost. The
slice keeps its place as the C2.5 direct-table lane with exact guards and full differential coverage; it is
not a speed claim, and public benchmark history is unchanged.

Fourteen ephemeral Droplets produced those three accepted gates. Eleven were rejected before or during
measurement - AMS3 on steal, AMS3 at a 36.342% within-order paired ratio spread against the 5% ceiling, NYC3
on steal, SFO3 at 5.098%, 6.689%, 11.403%, 27.537%, 17.578%, 12.997% and 8.591%, plus one accepted block
invalidated by steal at leg 7. This account offers only shared Basic plans, so region choice and re-drawing
were the available levers. Every Droplet and temporary key was confirmed deleted, and 225.2 billable minutes
cost about $0.31. Evidence for all three gates is under `bench/history/evidence/`; the first two directories
name commits that the tape rebase removed from the branch, and are retained only for their raw samples.

The measurable dense-table opportunity is array append and growth, where 100% of the suite's numeric array
writes actually land - not existing-slot mutation. Capacity prediction and append fast-pathing are a separate
slice with their own gate and must not be folded into this one.


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
