# VM Core 2 — interpreter parity without a JIT

> **Status (2026-08-24): C2.1 was remotely falsified; C2.2 Region-integrated private receipts are
> implemented, correctness-validated, and remotely accepted with 8.1651% lower ns/unit.** This is the execution plan for the largest safe non-JIT performance programme. It refines the
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
The next C2.2 slice is a mode-selected whole-function plain executable tape.
It should replace Region/receipt production metadata, predecode operands
and direct branch targets into effect-terminated blocks, and side-exit to the
unchanged scalar instruction before any mutation on a guard miss. Metered and
instrumented modes remain scalar initially. This attacks the repeated
executor-selection ladder across arbitrary verified blocks before the larger
frame-arena rewrite.

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

### C2.5 — direct tables and fixed-result calls

- Fast-path existing dense-slot mutation with incremental memory accounting and mutation-version updates.
- Add constant numeric slot forms and colocated PIC records.
- Extend exact-identity FASTCALL lanes and direct fixed-result writes without weakening monkey-patch fallback.
- Audit maintained memory counters against a debug full census after every mutation shape.

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
