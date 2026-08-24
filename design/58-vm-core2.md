# VM Core 2 — interpreter parity without a JIT

> **Status (2026-08-24): C2.1 implemented; correctness validation and controlled DigitalOcean measurement
> pending.** This is the execution plan for the largest safe non-JIT performance programme. It refines the
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

### C2.1 — stop dispatching avoidable register work (implemented)

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

Promotion gate: full formatting/clippy/workspace tests, all conformance fixtures, fuzz workspace, native/wasm
goldens, then an exact same-host DigitalOcean A/B. `generic_iteration` is the primary timing workload because it
keeps its dominant generic-iterator loops in the scalar/block tiers and has the strongest expected RegisterBlock
coverage; `dense_tables` is a secondary combined-tier breadth check if the budget permits.

### C2.2 — make blocks the baseline executor

- Introduce effect-terminated basic blocks with predecoded operands, logical cost, prefix costs, and liveness.
- Split entry into plain, metered, and instrumented modes while sharing semantic helpers.
- Move the common numeric/array lanes into direct block operations instead of nested Region dispatch.
- Require on/off differential tests for every opcode family, collection interval, fuel boundary, guard exit, and
  structured error PC.

### C2.3 — complete compiler O2 and cleanup liveness

- Add commit-late local compound assignment, target-directed nested lowering, constant/RK arithmetic and index
  forms, branch folding, and local-slot tightening.
- Replace uncaptured temporary clears with liveness receipts; preserve explicit close/detach operations for
  captured locals.
- Make block logical receipts the optimization-independent fuel schedule before calling this phase complete.

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
