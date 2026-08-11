# rustia — SotA performance research update

> **Status: [PERFORMANCE RESEARCH v2.0] (updated 2026-07-23).** This note updates
> [`55-improvements.md`](55-improvements.md) with the current performance research stack. It is not a new
> conformance contract; it is the ordered implementation bet list for making rustia fast while keeping the public
> workspace under `unsafe_code = "forbid"`.

## Ground truth

The public cross-engine benchmark posture is still conservative: rustia remains behind `mlua-luau` and Luaur
on every workload. `mlua-luau` wraps the reference C++ runtime; Luaur is a separate pure-Rust Luau
implementation. The current exact-commit
`34f6deff4a42501b6d66d257249eecfcbca1976a` Linux matrix places Luaur 2.615x-7.135x faster by minimum time with a
4.336x five-workload minimum-time geometric mean, and `mlua-luau` 2.615x-5.880x faster by minimum time with a
3.944x geometric mean. Rustia beats the Rust-native Ruau adapter on three of five workloads by minimum time. The snapshots used separate but identically specified dedicated instances, so movements are follow-up
signals rather than same-host causal attribution. All timed groups passed on their first attempts, and the
Linode was confirmed deleted before promotion. Exact same-engine Linux A/B evidence for earlier slices is
retained separately; none of these results turns into a blanket cross-engine headline. This remains a catch-up
project first.

Production prerequisites now implemented include complete object/program reclamation/reuse, quotas/OOM,
epoch-safe decision-reset ABI v1, maintained live-memory accounting, and the shared 96-epoch/3,072-wave
native/Node-V8 mixed-heap correctness gate. GC tuning may now use that gate, while broad memory-efficiency
claims still require external allocator/RSS evidence.

The hardened safe table/atom/cache foundation is now in-tree. VM-local byte atoms and insertion-ordered table
hash parts at 16+ entries use deterministic open-addressed `Vec<u32>` side indices at no more than 50% load.
They store indices into authoritative bytes/entries rather than duplicate keys; the table index is released on
clear or contraction below the threshold. Loaded programs cache constant atoms in compact `u32` slots and use a
4-byte site-map entry per PC. Eligible sites allocate 16-byte atom/slot/shape records only when filled, revalidate
the exact key and shape, and always read the live table value.

The compiled constant pool is now immutable shared storage, so cloned programs retained by separate VMs share
the same compiled string bytes. Long compiled string atoms reference that pool instead of allocating a second
per-VM byte vector; an equal runtime-owned atom is upgraded to the shared compiled storage if it arrived first.
Separately, ordinary compiled literals and runtime-produced binary strings up to 13 bytes can live directly
inside the sealed 16-byte `Value`; constant-key/cache paths still atomize them when identity is required. A short
key is canonicalized into an atom only when a successful non-nil table insertion needs it;
lookup misses, nil writes, readonly writes, and invalid-table writes use lookup-only canonicalization and do not
grow the atom table.

Mutation-safe `GETIMPORT` and generic fixed-result `FASTCALL` are also in-tree. The import descriptor supports
one to three components (the compiler currently lowers unshadowed two- and three-component dotted paths), reads
each component live, and falls back through ordinary index/metamethod behavior. `FASTCALL` guards the exact
current function handle and native function kind; a mismatch, reassignment, non-native callee, or metamethod
callee takes the ordinary resumable call path. Compiler prediction is limited to unshadowed standard-library
fields and binding-scoped aliases, so a shadowing local or parameter does not admit an unrelated call site.
Neither cache stores a final value or result.

Constant-key table and method specialization is now in-tree too. Non-import dot fields, constant-string bracket
operations, assignment targets, and named/constant-key constructors lower through `GETTABLEKS`/`SETTABLEKS`;
`GETTABLEKS` fuses the constant load and read at logical fuel 2. Compiler-emitted writes retain their one-fuel
key load before an intervening RHS, then use a one-fuel preloaded-key `SETTABLEKS`; the no-preload fully fused
bytecode mode costs 2. Both use the live slot cache on hits and misses. A compound update uses `GETTABLEKS`'s optional key-value output and a later `SETINDEX`, so its single
logical constant load is reused and the unfused three-step load/read/write fuel schedule is retained.
`NAMECALL` fuses method-name loading and lookup but intentionally leaves the subsequent `CALL` separate. Its
direct-receiver lane and live table-valued-`__index` lane re-resolve the current receiver metatable and `__index`
target, validate the key/shape/slot, and read the current callee; they do not cache a final callee, receiver, or
class. Lookup-only mode costs fuel 2, while the mode that folds the prior missing-method check costs 3 and retains
the same diagnostic. Full fused costs are charged before side effects; preloaded writes retain the original
cross-RHS fuel/GC phase, and synthetic instruction/GC steps preserve the unfused cadence. Function-valued
`__index` and all other fallback cases still use ordinary metamethod
semantics; this opcode is a lowering fast path, not a claim of full Luau `__namecall` context support.

The remaining avoidable costs include small-table scans, no fused
`GETIMPORT`+`FASTCALL` form, remaining dispatch and register tolls, no shared frozen standard-library state, and
no type-directed hot tier. A profiled hot-loop candidate now bypasses `gc-arena` when allocation debt is already
zero and refreshes weak-table root snapshots only before a possible collection boundary or guest-code entry.

The no-unsafe policy is a feature, not a speed dead-end. The product path below assumes safe Rust in the public
workspace. Techniques that require writable executable memory, raw code patching, or unsafe FFI remain research
references unless they can be kept outside the product gate or re-expressed safely.

## Top 10 methods, ordered by expected gain

**1. Type-directed wasm/AOT hot-kernel tier.**
Emit specialized wasm for hot, type-stable functions, either ahead-of-time or after a hotness threshold. Use guards
and deopt back to bytecode for dynamic cases. This is the best "better than C++ Luau interpreter" path because it
turns stable numeric/table kernels into direct wasm code without native executable memory or Rust `unsafe`.
Current plan delta: the table/IC base has landed. Production promotion of the validated
[`65-wasmjit.md`](65-wasmjit.md) direction waits for bounded memory/reset, semantic parity, guard/deopt, and
interpreter-equivalence gates rather than more one-off kernel code.
Expected gain: 10x-100x on monomorphic kernels; on those kernels rustia can plausibly beat the C++ Luau
interpreter by multiples. General Luau code still depends on guard quality and fallback cost.

**2. Harden and extend the landed table/atom/slot-cache foundation.**
The first slice now provides dense array storage, insertion-ordered hash entries, thresholded deterministic
indexed lookup, indexed VM-local atoms, compact per-loaded-program constant atoms, and live predicted string
slots on existing global/index/import opcodes. The first memory hardening pass replaced duplicate-key indices
with compact `u32` slots, reduced the table index to at most 50% load, added a 4-byte per-PC site map with
16-byte filled live-slot records, and stopped a monomorphic site from thrashing across atom changes. Shared
compiled constant storage, 13-byte inline literal/runtime strings, constant-key `GETTABLEKS`/`SETTABLEKS`, and two-lane
live `NAMECALL` lookup have now landed. A first focused hot-loop-hygiene correction also moves ordinary fuel
classification to a base-cost lookup and gives numeric table access and ordering number-first paths. Cached
length boundaries were implemented and exactly gated, then reverted after the accepted protocol measured no
win on either length-heavy workload (see the falsified-bet evidence below); next profile the remaining
small-table scan and dispatch costs.
Current plan delta: implementation and the controlled dense-table material-regression gate are complete. The
accepted exact-commit Linode v2 comparison measured 2.42% lower corrective-candidate ns/unit, with a conservative
nominal 95% interval of -3.35% to -1.57%, and published three actual final-binary rows plus raw evidence. That
resolves the predeclared +5% dense regression question for this workload and host protocol. A coherent
multi-engine, multi-workload snapshot now exists; broader public claims still require longer history and the
eventual approval of the existing generated per-workload claim gate.
Expected gain: 2x-20x on table-heavy workloads; likely the fastest route to closing the current benchmark gap.

**3. Adaptive quickening and specializing bytecode.**
Start with generic opcodes, observe stable operand types/keys/call targets, then rewrite the executing program or
per-VM side metadata to specialized forms with guards. This is the PEP 659-style "make the interpreter learn"
layer, adapted to rustia's deterministic fuel model.
Current plan delta: the first private `HotIr` tracelet slice is landed for bounded numeric `for` bodies. It keeps
public bytecode and the interpreter authoritative, lowers source-ordered typed operations, stages all fallible
work before its single allowed table effect, and falls through to the already-fetched interpreter instruction on
a guard miss. Next extend the same verified IR to multiple effects, branches, intrinsics, calls, and iteration;
do not revive the rejected per-site cache shape or add another bespoke recognizer.
Expected gain: 1.25x-2x broad speedup, larger when it removes repeated tag/key/call checks.

**4. FASTCALL, GETIMPORT, and safeenv completeness.**
The first safe slice is implemented: live one-to-three-component import resolution plus exact-handle native
call prediction for generic fixed-result calls. Standard-profile monkey-patching and host replacement remain
observable because imports cache only guarded locations and FASTCALL dispatches directly only when the current
function handle matches; every other case uses the ordinary index/call machinery. Import fuel is `2n - 1`
(1/3/5) on hit or miss, matching the unfused lowering's total cost and reserved atomically before the fused
lookup starts. FASTCALL costs the same one base fuel as CALL and charges native variable-work fuel exactly once.
Current plan delta: the combined slice is covered by the exact matrices. Profile it further, then consider safe
`GETIMPORT`+`FASTCALL` fusion where it
preserves those guards and fuel. Constant method lookup now has the separate `NAMECALL` fast path described
above; it does not fuse `CALL` or broaden the older dedicated math/string opcodes. The guarded mutation guarantee
applies to sites lowered through `GETIMPORT`/`FASTCALL`; those older opcodes retain their existing source-level
safeenv gates, and host-time replacement for them remains separate hardening work.
Expected gain: 2x-10x on stdlib-heavy code; essential for matching Luau on `stdlib_math_table`.

**5. Hot-loop hygiene: reduce dispatch count and per-instruction tolls.**
Move coverage, fuel, GC debt, root maintenance, and register access checks out of the common one-instruction path
where correctness permits. Use deterministic fuel batching at basic-block boundaries, direct register windows,
fallible slow helpers, and hot/cold opcode layout.
Current plan delta: the first focused pass removed repeated special-opcode comparisons from ordinary fuel
classification and avoided string/kind decoding on the dense numeric table path. The subsequent profiled pass
added an inlined zero-debt GC fast path and replaced unconditional per-instruction weak-root rescans with conservative
collection-boundary and guest-entry synchronization. Fuel, collection boundaries, metamethod entry, and explicit
`collectgarbage` behavior are unchanged. The exact `87db17542e19ae77ec92463f8a5b8e8b8a21a666` dedicated-Linode
matrix accepted every native and wasm timed group on its first attempt with zero timed steal. Relative to the
previous committed Linux snapshot, rustia's median ns/unit fell 34.97% on `dense_tables`, 27.20% on
`sparse_tables`, 27.44% on `closures_upvalues`, 22.08% on `generic_iteration`, and 22.77% on
`stdlib_math_table`. At that snapshot, the generated minimum-time claim gate placed `mlua-luau` 5.25x-9.53x faster, with a
7.30x geometric mean instead of 10.1x. The ephemeral VM was confirmed deleted before the evidence and 50-row
matrix were promoted. The next exact matrix at `b1cd2841fb8fc2d916047391a1d757d53a1303cd` adds the focused
numeric default-sort guard: dense-table median time improved another 8.90% versus that snapshot and its
`mlua-luau` gap moved from 9.20x to 8.54x. All timed groups passed on their first attempt; one native steal tick
(0.0118%) stayed inside the frozen allowance, and the VM was confirmed deleted before promotion.
Expected gain: 1.2x-3x broad speedup, especially on arithmetic, loops, and small functions.

**6. Superinstructions and tracelet fusion.**
Fuse common bytecode runs such as GETIMPORT+FASTCALL, LOADK+arithmetic, numeric-for loops, table append, and
method-call shapes. On wasm this is especially valuable because it reduces `br_table` dispatch count without
depending on unsafe/native threading.
Current plan delta: existing superinstruction work is correct; extend it from a few generic fused ops into a
profile-driven fusion set sourced from benchmark traces.
Expected gain: 1.2x-3x broad speedup; higher on small-loop wasm workloads.

**7. Luau O2-equivalent bytecode optimizer.**
Add constant folding, builtin folding, branch simplification, local slot tightening, import pre-resolution, and
typed lowering that feeds both the interpreter and wasm/AOT tier. This should mirror Luau's optimizer where
compatibility demands it and specialize more aggressively only when guards/deopts preserve semantics.
Current plan delta: present as a future compiler idea, but it needs to become part of the performance gate.
Expected gain: 1.1x-2x generally, with much larger wins on code that currently repeats avoidable work.

**8. Call, closure, and upvalue fast paths.**
Use short call frames, stack-resident temporaries, cheap open-upvalue representation, closure caching for
non-capturing or constant-capture closures, and direct internal stdlib calls. Avoid host-value round trips inside
the VM.
Current plan delta: the borrowing internal host ABI is adopted. The compiler now discovers captured bindings
before final lowering so immutable captures can be stored directly while mutable loop/backedge captures keep
cell semantics; a four-lane call-site PIC is keyed by program/prototype rather than exact closure identity. The
first dedicated-Linux closure gate landed; next profile frame/result construction, recursion/coroutine paths,
and small-function inlining without weakening policy/fuel/GC/coverage fallback.
Expected gain: 1.2x-4x on closure/upvalue/coroutine-shaped workloads.

**9. Allocation, GC, and string model overhaul.**
Use per-program intern pools, short-string storage, shared frozen stdlib state, and profile-specific GC
scheduling: allocation-debt pacing for Standard/LuauCompat, instruction-count boundaries for DeterministicSim.
Keep non-moving handles friendly to inline caches, but keep the VM thread-affine and safe.
Current plan delta: VM-local indexed atoms and per-loaded-program constant caches have landed. Shared cross-VM
compiled constant storage and the 13-byte inline literal/runtime-string form have now landed too. Lookup-only
short-key misses avoid permanent atom growth. Complete reachability/sweep, public retained roots,
generation-safe object/program reuse, maintained live/peak accounting, aggregate quotas and atomic OOM, plus
epoch-safe decision reset are implemented. The native/Node-V8 mixed-heap gate now covers 3,072 allocation waves
and stable warmed wasm linear memory; tune pacing without weakening those invariants. Shared frozen stdlib state
follows.
Expected gain: 1.2x-3x speed plus large RSS/startup wins for many-VM usage.

**10. Typed lanes for vectors, buffers, and deterministic extension kernels.**
For the deterministic-sim profile, expose generic typed lanes and buffer/vector operations that let hot numeric
work stay out of dynamic table/value traffic. Keep this public-clean by making the extension mechanism generic and
capability-checked rather than baking a private game opcode into the public VM.
Current plan delta: keep this behind the deterministic extension contract; use it for benchmarked sim kernels
after the generic engine is healthy.
Expected gain: workload-specific and potentially order-of-magnitude, but it does not replace the generic Luau
catch-up work.

## Measurement protocol for this slice

### Exact `fa8c857` Linux matrix — accepted, broader target revealed

The latest safe-Rust interpreter pass combines guarded common numeric and array
opcode lanes, direct exact built-in iteration and scalar math calls, compact
inline register/return/upvalue storage, cached allocation debt, exact weak-GC
countdowns, and complete-prototype leaf plans for the small closure shapes found
by profiling. Every shortcut falls back before observable mutation when its
guards do not hold; fuel, coverage, recursion, coercion, metamethod, and
collection-boundary tests exercise the deopt paths.

The earlier developer-host screen compared rustia only with the Rust-native Ruau
adapter and therefore selected a useful candidate against the wrong headline
reference. The accepted dedicated-Linux cohort measures all four Luau-family
adapters together. Rustia minimum times are 175,068/159,265/590/22,076/2,271
ns/unit for dense/sparse/closures/iteration/stdlib. Ruau is
129,662/154,867/562/29,840/2,143, so rustia wins iteration but not the other four.
Luaur and `mlua-luau` are substantially faster across all five, with roughly
4.93x geometric-mean advantages.

That result ruled out more benchmark-shape leaf recognizers as the primary
strategy. At that point, the next broad slice was a validated-profile quickened VM tier: move
hot table storage out of the `gc_arena` root container into direct `VmCell`
storage with explicit tracing, then emit guarded numeric arithmetic/loop and
numeric-array get/set forms. Fuel, coverage, weak-GC, and error policy checks
should move to explicit checkpoints that deopt to the semantic loop. This is the
cross-cutting path that can reduce the repeated arena-mutation, borrow, handle,
and dispatch tolls visible in all five workloads. The later direct `VmCell` stores and bounded private HotIR
implemented the first part of that direction; the current action queue is the lifecycle-first sequence above.

The harness now creates fresh prepared engine/workload state for each calibration probe and each recorded
repetition. Each recorded instance runs one untimed warmup followed by exactly one timed invocation, preventing
growing VM state and caches from leaking across samples while keeping setup/compilation outside the timer.

`--samples-json <path>` writes the execution-ordered `ns/unit` repetitions to a separate
`rustia.bench.raw-samples.v1` sidecar with commit, target, workload, selected `n`, fixed-versus-calibrated
selection, checksum, and summary metadata. It deliberately does not change the stable append-only history
schema, which retains min/median summaries.

For small engine changes, `--fixed-n <positive integer>` bypasses calibration and uses exactly the selected work
for every warmup and recorded repetition. Build baseline and candidate from byte-identical harness source, retain
separate raw sidecars, and alternate ABBA then BAAB order so drift is visible. For public history, measure the
accepted clean final-commit binary at least three independent times on the same controlled target, append only
the actual harness-produced summary rows, retain sidecars as separate evidence, and regenerate the history-derived
results and claims. `bench/analyze-fixed-samples.js` validates all fixed-work
sidecars and emits deterministic pooled and leg-clustered hierarchical bootstrap
intervals, with a conservative-envelope verdict against the predeclared
material-regression threshold; accepted evidence retains that exact JSON
output. The exact commands and required provenance are in
[`../bench/README.md`](../bench/README.md).

### GC idle/root synchronization candidate — local directional evidence

Symbolized optimized sampling on the allocation-light closure workload attributed 7.1% of baseline samples to
`gc_arena::context::Context::do_collection`, even though most instructions had no outstanding allocation debt.
The explicit zero-debt check removed that sampled bucket. A deliberately non-semantic diagnostic that omitted
the per-instruction weak-root refresh then established a roughly 25%-30% upper bound for the remaining root-copy
toll, so the production candidate keeps snapshots before any imminent weak-collection boundary and before every
opcode path that can enter guest code. A targeted test creates a weak-value table inside a nested call and proves
that a value live only in its caller register remains rooted; the complete 692-fixture conformance corpus also
passes.

A local optimized fixed-work ABBA/BAAB screen compared the clean pre-candidate binary with the complete candidate
over nine repetitions per leg and identical checksums. Averaging the two medians per variant gave directional
reductions of approximately 32% for `dense_tables`, 32% for `sparse_tables`, 29% for `closures_upvalues`, 27% for
`generic_iteration`, and 25% for `stdlib_math_table`. This developer-host screen selects the candidate for a
dedicated Linode matrix; it is not retained benchmark history or a public speed claim.

### Current field/string slice — exact-final dense comparison accepted

The accepted Linode v2 run compared baseline `dac4ec2295f77587c79f04e4ff0a09cb99912f96` with exact candidate
`ed65077c2e779ec6a641b8ca2f8c8258b2508a2d`, using byte-identical harness, workload, lockfile, and toolchain
inputs, fixed `n=1500`, 15 repetitions per leg, and the checksum
`number:193302491.00000000000000000`. Qualification spread was 0.4999%, all timed legs had zero contaminants,
and maximum timed steal was 0.0742%, below the prospectively declared 0.1% ceiling. The pooled baseline and
candidate medians were 310,170.536 and 302,665.974 ns/unit respectively, a -2.4195% candidate delta. The
conservative nominal 95% bootstrap interval was -3.3478% to -1.5699%, independently excluding the predeclared
+5% material regression. Three final candidate medians (303,918.403, 303,335.093, and 300,979.767 ns/unit) were
appended to public history with their raw sidecars and full gate/comparison evidence. The conclusion is limited
to this exact dense workload and Linux protocol; it is not a blanket speed claim.

Static bytecode and release-disassembly analysis showed that `dense_tables` executes none of the new
`GETTABLEKS`, `SETTABLEKS`, or `NAMECALL` opcodes. Its candidate lowering dispatches 6,964 instructions per
unit versus 6,967 for the baseline, so the small diagnostic disadvantage was not caused by added bytecode
dispatch. The cross-cutting candidates were instead ordinary opcodes traversing special fuel comparisons,
numeric table accesses probing the short-string representation first, and numeric ordering repeating kind
decoding. The corrective pass uses a 256-entry base-cost lookup with validated zero-sentinel special cases,
number-first table get/set/order paths, and exact-tag nil checks. Logical fuel, invalid-program validation,
numeric-key rules, and string canonicalization remain unchanged. Semantic fused-op costs stay deterministic, but
aggregate GC checkpoints can move when specialization removes dead cleanup bytecode. The controlled
exact-commit result accepts this corrective pass for the predeclared dense material-regression gate.

### OOP/GC fast-path candidate — exact-final comparison accepted

The second accepted Linode v2 run compared baseline `ae3beae2025feb21d0e37c70168204ae154ae125` with exact
candidate `f208c12d0ce266117743569d736346fae9a35db2` on `oop_metamethods`, using fixed `n=24000`, 15 repetitions
per leg, four ABBA/BAAB legs per build, and the predeclared zero-regression threshold. Its pooled baseline and
candidate medians were 21,633.014 and 7,364.006 ns/unit, a -65.9594% candidate delta. The conservative nominal
95% interval was -66.3876% to -65.5482%, excluding zero; three actual final-candidate rows and every raw/gate
artifact are retained under
[`../bench/history/evidence/f208c12d0ce266117743569d736346fae9a35db2/`](../bench/history/evidence/f208c12d0ce266117743569d736346fae9a35db2/).
The comparison covers the complete candidate, including its constant-field/method paths and a correction that
avoids repeated weak-table scanning when no `__mode` atom exists. It therefore demonstrates the exact candidate
and workload, not that one opcode family alone caused the full delta.

### Shared/inline strings — exact 512-VM memory evidence completed

The evidence-only Linode run compared the same baseline with exact candidate
`1370d6ccefba7a1956441247c3139af2a2cf31fa`. One ten-sample round warmed the host and was excluded. Sixteen
accepted paired rounds then sampled five scenarios for both builds in 160 unique fresh processes, each creating
512 VMs around a 1,024-constant program. Round 14's first attempt observed one steal tick and discarded all ten
samples before its clean retry. Primary memory is `/proc/<pid>/smaps_rollup` anonymous growth from program-ready
to after-run, analyzed as paired scenario contrasts in KiB/VM with a seeded 100,000-resample bootstrap.

| paired candidate-minus-baseline contrast | median KiB/VM | nominal 95% paired interval | candidate residual |
|---|---:|---:|---:|
| unloaded short constants minus empty | -32.0078 | -32.0156 to -32.0078 | approximately 0 KiB/VM |
| unloaded long constants minus empty | -258.0000 | -258.0000 to -258.0000 | 0 KiB/VM |
| loaded short minus unloaded short | -105.3906 | -105.3984 to -105.3906 | 0 KiB/VM |
| loaded long minus unloaded long | -260.9688 | -260.9766 to -260.9609 | 7.0781 KiB/VM |

Corroborating private-dirty/RSS fields agree with the primary contrasts. Descriptive medians show the candidate's
one-time short-program image was 4 KiB larger and program construction was slower in the sampled scenarios, while
combined VM creation plus execution was faster in every scenario; those timing summaries do not have the paired
confidence intervals used for the primary memory contrasts. The corpus is
retained under
[`../bench/evidence/vm-string-memory/1370d6ccefba7a1956441247c3139af2a2cf31fa/`](../bench/evidence/vm-string-memory/1370d6ccefba7a1956441247c3139af2a2cf31fa/)
and intentionally does not append throughput history. It quantifies the shared/inline string slice on this Linux
fixture, not a general mixed-heap or wasm RSS budget.

The first six ABBA/BAAB legs were usable. Pooling their 45 samples per variant gave baseline median
139,964.195 ns/unit (IQR 133,262.333–156,530.778) and candidate median 141,223.833 ns/unit (IQR
132,568.028–160,640.917), a +0.90% difference whose leg signs flipped and whose distributions overlap heavily.
Two subsequent legs measured 978,686.639 and 825,969.167 ns/unit while unrelated release builds drove host load
into triple digits; both were excluded as externally contaminated rather than selected by outcome. These
temporary sidecars and the `ae3beae-working-tree` candidate label remain diagnostic only. The later accepted
exact-commit Linode sequence supersedes them for the dense material-regression decision; neither the older
+1.29% observation nor the +0.90% working-tree diagnostic is reinterpreted as accepted evidence.

### 2026-07-17 local hardening measurement

The final candidate was compared with baseline `dac4ec2295f77587c79f04e4ff0a09cb99912f96` using byte-identical
fresh-state harness source (SHA-256 `54dca797295afc9e2a648d266b19b82b06cbc1d31528ddf507829fac5ca99722`).
Both release binaries used Rust 1.95.0 with `RUSTC_WRAPPER=`. The broad run order was
baseline→candidate→candidate→baseline, with 11 raw repetitions per leg, a 200 ms target, separate
`rustia.bench.raw-samples.v1` sidecars, and 22 pooled samples per variant. Lower is better:

| workload | baseline median ns/unit | candidate median ns/unit | delta |
|---|---:|---:|---:|
| `dense_tables` | 128,666.787 | 132,010.353 | +2.60% (noisy, overlapping IQRs) |
| `sparse_tables` | 78,062.878 | 73,718.446 | -5.57% (large leg drift) |
| `closures_upvalues` | 416.632 | 411.837 | -1.15% |
| `stdlib_math_table` | 1,146.112 | 1,063.043 | -7.25% |
| `string_building` | 105,131.495 | 84,411.644 | -19.71% |

Because sustained runs showed thermal drift, cooled single-workload ABBAs checked the two suspected regressions.
Closures measured 452.845 to 437.835 ns/unit (-3.31%); dense tables measured 124,125.052 to 125,727.459
ns/unit (+1.29%, with MAD 898.490/852.830 ns and nearly touching IQRs). The audit therefore found no remaining
material regression above 5%, while retaining the small dense-table disadvantage as a follow-up rather than
claiming neutrality.

The measurement itself exposed and removed a real pre-fix regression: transient `GETIMPORT` rooting had moved
the GC-debt check into an out-of-line call on every instruction, and ordinary `CALL` shared FASTCALL's optional
cache-site branch. The final release binary inlines the collection path (no helper symbol/call in `Vm::run_function`)
and keeps ordinary calls on their original helper. The raw sidecars were temporary `/private/tmp` artifacts and
were not retained in the repository or appended to public history; this summary is the durable evidence. The
result supports hardening decisions but does not authorize a blanket release speed headline. The projected gains
below remain planning estimates rather than measured claims against C++ Luau.

### Numeric default `table.sort` candidate — exact matrix accepted

Symbolized optimized sampling attributed approximately 10.6% of `dense_tables` leaf samples to generic raw
ordering inside the native heap-sort loop. The candidate recognizes a pair of numeric `Value`s at the sort
comparison boundary and performs the same partial-order `<` decision directly. String ordering, custom guest
comparators, mutation detection, errors, and all nonnumeric fallback behavior continue through the existing
paths.

A fixed-work local ABBA screen used nine repetitions per leg at `n=3000`, with byte-identical harness source
and matching checksums. Averaged minimum time moved from 73,953.28 to 64,815.11 ns/unit (-12.35%); averaged
median time moved from 74,663.19 to 66,236.49 ns/unit (-11.29%). This developer-host result selected the focused
two-line candidate for exact-commit Linux measurement. It is directional evidence, not retained public history
or a cross-engine claim.

The exact-commit `b1cd2841fb8fc2d916047391a1d757d53a1303cd` dedicated-Linode matrix then accepted every
native and wasm timed group on its first attempt. The primary native group observed one steal tick (0.0118%,
within the frozen 0.1% allowance); all other timed groups observed zero. Relative to the immediately preceding
exact Linux snapshot, rustia dense-table minimum time moved from 177,347.826 to 164,192.888 ns/unit (-7.42%)
and median time from 182,365.970 to 166,135.066 ns/unit (-8.90%). The generated dense `mlua-luau` minimum-time
gap moved from 9.20x to 8.54x. Independent-host variation in unaffected workloads left the five-workload
geometric mean effectively flat at 7.29x, with a 5.09x-10.02x range in that snapshot; no unrelated delta is attributed to
the sort guard. The VM was confirmed deleted after 1,364 seconds, before promotion, with a rounded $0.108
compute-charge ceiling. The complete corpus is retained under
[`../bench/evidence/matrix/b1cd2841fb8fc2d916047391a1d757d53a1303cd/`](../bench/evidence/matrix/b1cd2841fb8fc2d916047391a1d757d53a1303cd/).

An earlier persistent per-site numeric quickening experiment was rejected before this focused change: fixed-work
screens regressed representative dense, closure, and generic-iteration paths by roughly 3%-7%. A stateless
operand-preparation variant produced only mixed marginal results and was removed too. Neither experiment is in
the accepted commit; future quickening work needs a different dispatch/state design rather than reviving that
cache shape.

### Private HotIR numeric tracelets — landed and Linux-gated

The specialization pass at exact commit `d4ec275e19bad0efcf8e96db4030e7380bd93a9b` replaces the recursive,
bespoke `NumericLoopKernel` with a private, bounded `HotIr`. Public bytecode and the ordinary interpreter remain
the semantic authority. Complete eligible numeric
`for` bodies lower to source-ordered typed operations with forward-only number IDs; the executor evaluates each
operation once into reused unboxed `f64` scratch, stages every register and table operand, and commits only after
all guards succeed. A guard miss resumes the already-fetched `ForLoop` directly, so completed iterations are not
replayed. Fuel, coverage, allocation-debt, and deterministic-GC boundaries still prevent entry.

The flat ordering deliberately retains overwritten arithmetic and indexed reads instead of dead-code-eliminating
them. That preserves their possible type errors and metamethod calls while removing the old recursive expanded-DAG
limit: 128 source operations require exactly 128 scratch outputs even for repeated `x = x + x` shapes. The first
effect slice remains conservative: at most one numeric array write, after all fallible operations, followed only
by cell cleanup. Multiple writes will require a preflight/overlay transaction before HotIR grows that capability.

Profiling baseline `bd251d9eaf92e2088e8902fddad338a252e292ed` explained the redesign. Recursive expression
evaluation occupied 17.03% of leaf samples (21.71% inclusive), while consuming the inline staged-write `SmallVec`
triggered a 376-byte copy every accelerated iteration and occupied another 16.78%. The dense workload's two plans
have only four final register writes, yet copied about 192,512 bytes per benchmark unit. HotIR reuses both scratch
buffers and commits through borrowed iteration, eliminating that copy.

A local fixed-work eight-leg ABBA/BAAB screen used separate release binaries, identical harness source, seven raw
repetitions per leg, and matching checksums. Pooled median `dense_tables` time moved from 17,996.47 to 12,721.87
ns/unit (-29.31%). The same screen found -0.52% on `sparse_tables`, -3.65% on `closures_upvalues`, +0.66% on
`generic_iteration`, and -4.69% on `stdlib_math_table`; no tracked default crossed the predeclared +3% local
regression threshold. These macOS developer-host results are directional selection evidence, not a public Linux
or cross-engine claim.

The exact dedicated-Linode matrix accepted every native and wasm group on its first timed attempt. Against the
immediately preceding `3732b80d5200edf053e213ef931b838c003968f3` snapshot, rustia native median time changed
-0.23% on `dense_tables`, -7.14% on `sparse_tables`, +5.74% on `closures_upvalues`, +5.12% on
`generic_iteration`, and -0.59% on `stdlib_math_table`, for a +0.47% normalized geometric mean. The Linux result
therefore preserves the prior native tier but does **not** reproduce the local -29.31% dense gain; the local
number remains directional only. That snapshot's minimum-time gaps are 3.495x to `mlua-luau` and 3.757x to Luaur by
five-workload geometric mean, while rustia beats Ruau on all five. Rustia wasm improved 5.55% by normalized
geometric mean against the preceding snapshot, but cross-run movement is not attributed solely to HotIR. The VM
was confirmed deleted after 1,529 seconds and before promotion, with a rounded $0.108 compute ceiling. The
complete corpus is retained under
[`../bench/evidence/matrix/d4ec275e19bad0efcf8e96db4030e7380bd93a9b/`](../bench/evidence/matrix/d4ec275e19bad0efcf8e96db4030e7380bd93a9b/).

### Prototype-polymorphic closure calls — landed and Linux-gated

Exact commit `a5629199643847c0da78792644ad3656887bee21` adds direct immutable-upvalue captures and a four-lane
call-site PIC keyed by loaded program/prototype. Mutable captures, recursion, policy, guard, fuel, coverage, and
collection boundaries keep their ordinary cell/interpreter paths. Against the preceding exact snapshot,
`closures_upvalues` median latency moved from 270.427 to 229.766 ns/call: 15.036% lower latency and 1.177x
throughput. All five new raw samples (229.095-232.206 ns) beat all five prior samples
(269.927-272.955 ns); each cohort matched its own calibrated-work checksum.

The two cohorts used separate but identically specified dedicated Linode instances, so this is a strong
same-protocol result rather than a paired-host attribution. Other rustia median movements were +2.35% on
`dense_tables`, +12.70% on `sparse_tables`, +8.93% on `generic_iteration`, and +5.74% on
`stdlib_math_table`, for a +2.45% normalized five-workload geometric mean; treat those as stability signals and
recheck sparse/generic rather than assigning them to the closure change. Current minimum-time geometric-mean
gaps are 3.494x to `mlua-luau` and 3.833x to Luaur, and rustia beats Ruau on four of five workloads by minimum
time. The VM was
confirmed deleted before promotion with a $0.108 rounded compute ceiling. Evidence is retained under
[`../bench/evidence/matrix/a5629199643847c0da78792644ad3656887bee21/`](../bench/evidence/matrix/a5629199643847c0da78792644ad3656887bee21/).

### Cached length boundaries — exact gates accepted, bet falsified, reverted

Exact commit `34f6deff4a42501b6d66d257249eecfcbca1976a` cached each table's length border in a
`mutation_version`-stamped slot consulted by `#t` reads and by the internal border computations of
`table.insert`/`table.remove`; every mutation path invalidates by bumping the version. The slice also
required a fix bumping that version when quiescent spare-capacity release pops trailing array nils. A local
developer-host ABBA/BAAB screen was directionally +2.85% candidate ns/unit (hierarchical nominal interval
-0.19% to +4.79%); that screen is diagnostic only and not a public claim.

The first accepted Linode v2 run compared baseline `ed26fb2f26be2b7dc851e4ee043c4ccad56ac547` with the exact
candidate on `dense_tables`, using byte-identical harness, workload, lockfile, and toolchain inputs, fixed
`n=1500`, 15 repetitions per leg, four ABBA/BAAB legs per build, and checksum
`number:193302491.00000000000000000`. Qualification spread was 1.0041% against the 3% ceiling, and every
timed leg passed its idle gate with zero contaminants and zero steal. The pooled baseline and candidate
medians were 38,712.969 and 39,393.119 ns/unit, a +1.7569% candidate delta. The conservative nominal 95%
interval was +0.0238% to +3.6313% (pooled +1.1457% to +3.0797%), excluding the predeclared +5% material
regression but also excluding any dense win: constant mutation during the build phases keeps the version
counter advancing, so the cache almost never hits and only pays its validation cost. The VM was confirmed
deleted before export with a $0.054 rounded compute ceiling. The three remote final rows were intentionally
left unpromoted because public history admits only complete engine-times-workload cohorts; the targeted
evidence is retained in the run state.

The second accepted Linode v2 run compared the same baseline with `ef9e07078deb287af4ff08807bbdb839aff62f55`
(code-identical to the candidate, pinned to post-promotion history) on `sparse_tables`, fixed `n=1202`,
checksum `number:1506346400.00000000000000000`, and bootstrap seed 780221945. The pooled medians were
99,211.592 and 99,822.523 ns/unit, a +0.6158% delta whose conservative nominal 95% interval of -4.2547% to
+2.0325% (pooled -0.3966% to +1.6532%) straddles zero: statistically neutral on the workload whose border
churn was the bet's best case. All legs passed their idle gates with zero steal, and the VM was confirmed
deleted with a $0.054 rounded compute ceiling. An earlier attempt at this run failed a provenance check
(candidate-side history pin) before any measurement and was destroyed unused.

The exact-commit `34f6deff4a42501b6d66d257249eecfcbca1976a` cross-engine matrix then replaced the public
Linux snapshot (65 native rows, 15 wasm rows); all four timed groups were accepted on their first timed
attempt with zero timed steal, and the VM was confirmed deleted after 1,560 seconds with a $0.108 rounded
compute ceiling. Versus the preceding `83bc2f3` snapshot — 86 commits earlier, so these are cross-cohort
stability signals rather than single-change attribution — rustia median ns/unit moved -7.07% dense, +8.06%
sparse, +20.97% closures, +16.10% generic iteration, and +17.32% stdlib. Evidence is retained under
[`../bench/evidence/matrix/34f6deff4a42501b6d66d257249eecfcbca1976a/`](../bench/evidence/matrix/34f6deff4a42501b6d66d257249eecfcbca1976a/).

Verdict: the bet's premise — that length-border recomputation is a material, cacheable cost on table-heavy
code — is falsified on the two most length-heavy tracked workloads. The change was reverted at `74340c4`;
the spare-capacity mutation-version bump survives as a standalone correctness fix for the pre-existing
version consumers (iteration guards), pinned by a regression test. Cached length boundaries are closed;
reopen only with a workload that demonstrates cache hits in a measured hot phase.

## Estimate vs C++ Luau

| state | likely position vs C++ Luau interpreter |
|---|---|
| Current public cross-engine snapshot | The two faster Luau adapters are 2.615x-7.135x faster on the five tracked workloads, with 3.944x (`mlua-luau`, reference C++) and 4.336x (pure-Rust Luaur) geometric means. Rustia beats Rust-native Ruau on three of five by minimum time. |
| Full tables + shared atoms + broader ICs + FASTCALL/GETIMPORT | Could close most table/global/stdlib gaps; target 0.5x-1.0x Luau on many interpreter workloads. |
| Extend HotIR + hot-loop hygiene + superinstructions + O2 optimizer | Plausible 0.8x-1.5x Luau interpreter on favorable workloads, but only measurement can bless this. |
| Add type-directed wasm/AOT tier | Hot type-stable kernels can plausibly beat the C++ Luau interpreter by multiples; this is the main better-than-Luau-interpreter route. |
| Native copy-and-patch/JIT family | Research reference only for the no-unsafe product. Useful to study, not a public-workspace implementation plan unless the unsafe boundary policy changes. |

The honest target is therefore two-layered:

- **Interpreter parity first:** make safe-Rust rustia approach C++ Luau interpreter speed on broad Luau code.
- **Hot-tier advantage second:** use type-directed wasm/AOT to beat the interpreter on stable kernels without
  giving up the no-unsafe feature.

## Source notes

- [Luau performance](https://luau.org/performance/) — register VM, imports, FASTCALL, inline caching, tables,
  closure/GC/library techniques.
- [Roblox Luau native code generation](https://github.com/Roblox/creator-docs/blob/main/content/en-us/luau/native-code-gen.md)
  — type-guided native tier, guards, fallback behavior.
- [Deegen / LuaJIT Remake](https://arxiv.org/abs/2411.11469) and the
  [Deegen PDF](https://fredrikbk.com/publications/deegen.pdf) — generated VM/JIT architecture and modern
  interpreter/JIT reference point.
- [Copy-and-patch compilation](https://fredrikbk.com/publications/copy-and-patch.pdf) — native baseline compiler
  reference; not a no-unsafe public-workspace plan as-is.
- [PEP 659](https://peps.python.org/pep-0659/) — adaptive specializing interpreter design.
- [Lazy basic block versioning](https://arxiv.org/abs/1411.0352) — type-specialized JIT direction relevant to
  wasm/AOT guards.
- [The Structure and Performance of Efficient Interpreters](https://jilp.org/vol5/v5paper12.pdf) — dispatch cost
  and interpreter structure background.
