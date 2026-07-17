# rustia — SotA performance research update

> **Status: [PERFORMANCE RESEARCH v1.3] (updated 2026-07-17).** This note updates
> [`55-improvements.md`](55-improvements.md) with the current performance research stack. It is not a new
> conformance contract; it is the ordered implementation bet list for making rustia fast while keeping the public
> workspace under `unsafe_code = "forbid"`.

## Ground truth

The committed benchmark posture is still conservative: rustia is currently far behind the C++ Luau oracle
through `mlua-luau` on the tracked workloads, with the generated benchmark claim saying rustia trails by roughly
8x-20x and about 13x geomean in the committed snapshot. That makes this a catch-up project first.

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

The remaining avoidable costs include small-table scans, cached length-boundary work, no fused
`GETIMPORT`+`FASTCALL` form, heavy per-instruction book-keeping, no shared frozen standard-library state, and no
type-directed hot tier.

The no-unsafe policy is a feature, not a speed dead-end. The product path below assumes safe Rust in the public
workspace. Techniques that require writable executable memory, raw code patching, or unsafe FFI remain research
references unless they can be kept outside the product gate or re-expressed safely.

## Top 10 methods, ordered by expected gain

**1. Type-directed wasm/AOT hot-kernel tier.**
Emit specialized wasm for hot, type-stable functions, either ahead-of-time or after a hotness threshold. Use guards
and deopt back to bytecode for dynamic cases. This is the best "better than C++ Luau interpreter" path because it
turns stable numeric/table kernels into direct wasm code without native executable memory or Rust `unsafe`.
Current plan delta: promote the validated [`65-wasmjit.md`](65-wasmjit.md) direction from spike to first-class
performance tier after the table/IC base lands.
Expected gain: 10x-100x on monomorphic kernels; on those kernels rustia can plausibly beat the C++ Luau
interpreter by multiples. General Luau code still depends on guard quality and fallback cost.

**2. Harden and extend the landed table/atom/slot-cache foundation.**
The first slice now provides dense array storage, insertion-ordered hash entries, thresholded deterministic
indexed lookup, indexed VM-local atoms, compact per-loaded-program constant atoms, and live predicted string
slots on existing global/index/import opcodes. The first memory hardening pass replaced duplicate-key indices
with compact `u32` slots, reduced the table index to at most 50% load, added a 4-byte per-PC site map with
16-byte filled live-slot records, and stopped a monomorphic site from thrashing across atom changes. Shared
compiled constant storage, 13-byte inline literal/runtime strings, constant-key `GETTABLEKS`/`SETTABLEKS`, and two-lane
live `NAMECALL` lookup have now landed. Next add cached length boundaries and profile the remaining small-table
scan and dispatch costs.
Current plan delta: implementation is complete for this slice, but performance validation is not. The earlier
representation-hardening measurement found no credible material regression above 5% and kept a borderline
+1.29% dense-table result visible; the newer field/string changes still need fixed-work validation and committed
repeatable history before attributing a public speedup or claiming the dense follow-up is resolved.
Expected gain: 2x-20x on table-heavy workloads; likely the fastest route to closing the current benchmark gap.

**3. Adaptive quickening and specializing bytecode.**
Start with generic opcodes, observe stable operand types/keys/call targets, then rewrite the executing program or
per-VM side metadata to specialized forms with guards. This is the PEP 659-style "make the interpreter learn"
layer, adapted to rustia's deterministic fuel model.
Current plan delta: current docs mention generic superinstructions and ICs, but not a full adaptive tier. Add a
quickening state machine and fuel-invariant specialized opcode families.
Expected gain: 1.25x-2x broad speedup, larger when it removes repeated tag/key/call checks.

**4. FASTCALL, GETIMPORT, and safeenv completeness.**
The first safe slice is implemented: live one-to-three-component import resolution plus exact-handle native
call prediction for generic fixed-result calls. Standard-profile monkey-patching and host replacement remain
observable because imports cache only guarded locations and FASTCALL dispatches directly only when the current
function handle matches; every other case uses the ordinary index/call machinery. Import fuel is `2n - 1`
(1/3/5) on hit or miss, matching the unfused lowering's total cost and reserved atomically before the fused
lookup starts. FASTCALL costs the same one base fuel as CALL and charges native variable-work fuel exactly once.
Current plan delta: benchmark this combined slice, then consider safe `GETIMPORT`+`FASTCALL` fusion where it
preserves those guards and fuel. Constant method lookup now has the separate `NAMECALL` fast path described
above; it does not fuse `CALL` or broaden the older dedicated math/string opcodes. The guarded mutation guarantee
applies to sites lowered through `GETIMPORT`/`FASTCALL`; those older opcodes retain their existing source-level
safeenv gates, and host-time replacement for them remains separate hardening work.
Expected gain: 2x-10x on stdlib-heavy code; essential for matching Luau on `stdlib_math_table`.

**5. Hot-loop hygiene: reduce dispatch count and per-instruction tolls.**
Move coverage, fuel, GC debt, root maintenance, and register access checks out of the common one-instruction path
where correctness permits. Use deterministic fuel batching at basic-block boundaries, direct register windows,
fallible slow helpers, and hot/cold opcode layout.
Current plan delta: hot/cold split is already promoted, but the VM loop should get a focused profile pass before
larger compiler work hides the easy wins.
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
Current plan delta: the borrowing internal host ABI is adopted; extend the same thinking to all VM calls and
closures.
Expected gain: 1.2x-4x on closure/upvalue/coroutine-shaped workloads.

**9. Allocation, GC, and string model overhaul.**
Use per-program intern pools, short-string storage, shared frozen stdlib state, and deterministic allocation-driven
GC pacing. Keep non-moving handles friendly to inline caches, but keep the VM thread-affine and safe.
Current plan delta: VM-local indexed atoms and per-loaded-program constant caches have landed. Shared cross-VM
compiled constant storage and the 13-byte inline literal/runtime-string form have now landed too. Lookup-only short-key
misses avoid permanent atom growth. Shared frozen stdlib state and broader allocation/GC tuning remain because
tables, atoms, imports, and ICs compound.
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

### Current field/string slice — exact-final measurement pending

A fixed-work diagnostic has been run for the `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` and string-representation
working tree, but no accepted exact-final result or public-history row exists yet. Baseline
`dac4ec2295f77587c79f04e4ff0a09cb99912f96` and the working candidate used byte-identical harness source
(SHA-256 `9cb397349bf2e630e2ca120bd2b84b70aa83bf1dbeb496aa3f05622b2696325c`) and dense workload source
(`b0c048b9fb17582fa409ae46b977cc73da635c145df39ea6f8fd756e9e17118d`), fixed `n=1500`, 15 repetitions, and
the identical checksum `number:193302491.00000000000000000`.

The first six ABBA/BAAB legs were usable. Pooling their 45 samples per variant gave baseline median
139,964.195 ns/unit (IQR 133,262.333–156,530.778) and candidate median 141,223.833 ns/unit (IQR
132,568.028–160,640.917), a +0.90% difference whose leg signs flipped and whose distributions overlap heavily.
Two subsequent legs measured 978,686.639 and 825,969.167 ns/unit while unrelated release builds drove host load
into triple digits; both were excluded as externally contaminated rather than selected by outcome. These
temporary sidecars and the `ae3beae-working-tree` candidate label are diagnostic only. The complete sequence must
be repeated from hashed exact-commit binaries on an idle host, retained durably, and followed by three actual
final-commit history appends. Until then, neither the older +1.29% observation nor this +0.90% diagnostic supports
a speedup, neutrality claim, or dense-regression resolution.

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

## Estimate vs C++ Luau

| state | likely position vs C++ Luau interpreter |
|---|---|
| Current committed benchmark snapshot | About 8x-20x slower on tracked workloads; roughly 13x slower geomean. The newer field/string code is not represented yet. |
| Full tables + shared atoms + broader ICs + FASTCALL/GETIMPORT | Could close most table/global/stdlib gaps; target 0.5x-1.0x Luau on many interpreter workloads. |
| Add quickening + hot-loop hygiene + superinstructions + O2 optimizer | Plausible 0.8x-1.5x Luau interpreter on favorable workloads, but only measurement can bless this. |
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
