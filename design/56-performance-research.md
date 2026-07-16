# rustia — SotA performance research update

> **Status: [PERFORMANCE RESEARCH v1.1] (updated 2026-07-16).** This note updates
> [`55-improvements.md`](55-improvements.md) with the current performance research stack. It is not a new
> conformance contract; it is the ordered implementation bet list for making rustia fast while keeping the public
> workspace under `unsafe_code = "forbid"`.

## Ground truth

The committed benchmark posture is still conservative: rustia is currently far behind the C++ Luau oracle
through `mlua-luau` on the tracked workloads, with the generated benchmark claim saying rustia trails by roughly
8x-20x and about 13x geomean in the committed snapshot. That makes this a catch-up project first.

The first safe table/atom/cache foundation is now in-tree. Dense-array plus insertion-ordered hash tables use a
deterministic open-addressed side index at 16+ hash entries; VM-local byte interning is indexed by `BTreeMap`;
each loaded program caches its VM-local string-constant atoms; and existing global/`GetIndex`/`SetIndex` paths
have positive per-program/function/PC string-slot caches. Those caches store atom+slot only, revalidate
shape/key/live value, preserve metamethod and readonly behavior, and do not change opcode fuel.

The remaining avoidable costs include small-table scans, no shared cross-VM `Program` string pool or
short-string inline form, no dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` tier, limited FASTCALL/GETIMPORT
coverage, heavy per-instruction book-keeping, and no type-directed hot tier.

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
indexed lookup, indexed VM-local atoms, per-loaded-program constant-atom caches, and positive predicted string
slots on existing global/index opcodes. Next add cached length boundaries, shared program string storage where
safe, and dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` specialization.
Current plan delta: measure and harden the landed slice before broadening it; retain the safe side-metadata and
fuel-invariant rules.
Expected gain: 2x-20x on table-heavy workloads; likely the fastest route to closing the current benchmark gap.

**3. Adaptive quickening and specializing bytecode.**
Start with generic opcodes, observe stable operand types/keys/call targets, then rewrite the executing program or
per-VM side metadata to specialized forms with guards. This is the PEP 659-style "make the interpreter learn"
layer, adapted to rustia's deterministic fuel model.
Current plan delta: current docs mention generic superinstructions and ICs, but not a full adaptive tier. Add a
quickening state machine and fuel-invariant specialized opcode families.
Expected gain: 1.25x-2x broad speedup, larger when it removes repeated tag/key/call checks.

**4. FASTCALL, GETIMPORT, and safeenv completeness.**
Implement Luau-style import resolution and builtin fast paths so `math.*`, `table.*`, string methods, and common
stdlib calls avoid global lookup, avoid ordinary call-frame setup, and deopt correctly when the Standard profile
mutates the base environment.
Current plan delta: already adopted in `55-improvements`; make it a benchmark-blocking performance feature, not a
later cleanup.
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
program strings, short-string storage, and frozen stdlib state remain representation work because tables, atoms,
imports, and ICs compound.
Expected gain: 1.2x-3x speed plus large RSS/startup wins for many-VM usage.

**10. Typed lanes for vectors, buffers, and deterministic extension kernels.**
For the deterministic-sim profile, expose generic typed lanes and buffer/vector operations that let hot numeric
work stay out of dynamic table/value traffic. Keep this public-clean by making the extension mechanism generic and
capability-checked rather than baking a private game opcode into the public VM.
Current plan delta: keep this behind the deterministic extension contract; use it for benchmarked sim kernels
after the generic engine is healthy.
Expected gain: workload-specific and potentially order-of-magnitude, but it does not replace the generic Luau
catch-up work.

## Estimate vs C++ Luau

| state | likely position vs C++ Luau interpreter |
|---|---|
| Current committed snapshot | About 8x-20x slower on tracked workloads; roughly 13x slower geomean. |
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
