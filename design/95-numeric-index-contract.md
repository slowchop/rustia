# Numeric string indices

## Scope and decision

The shared index helper used by `string.sub` uses a checked signed 64-bit conversion in
all four profiles. Keep this deterministic rule; do not reproduce unchecked
machine conversions observed in one Luau oracle build. This document specifies
the numeric conversion, not every builtin that accepts an integer.

1. Apply the builtin's existing numeric coercion and argument validation.
2. Truncate a finite number toward zero.
3. Accept the truncated result exactly when `-2^63 <= result < 2^63`.
4. Reject NaN, positive infinity, negative infinity, and out-of-range results
   with the existing structured type error (`integer string library index`).
5. Apply the builtin's normal positional rules after conversion. For
   `string.sub`, negative positions count from the end, bounds are clipped,
   and an inverted interval returns the empty string. Missing or nil end
   positions keep their normal default.

The upper comparison must be exclusive: converting `i64::MAX` to `f64` rounds
up to `2^63`. A comparison against that rounded value using `>` admits an
out-of-range value and Rust's saturating cast hides the error. The correct
upper test is `>= 2^63`, equivalently `> (2^63 - 1024)` for f64 values;
the implementation uses a half-open range check before the truncating cast.
`-2^63` remains representable and valid. The adjacent
representable doubles are `2^63 - 1024` below the upper bound and
`-2^63 - 2048` below the lower bound.

This preserves existing in-range signed-64 behavior and nonfinite rejection.
It deliberately does not promise Luau parity for conversions outside the
representable range of Luau's C++ `int`. It does not change fuel, profiles,
argument evaluation, source diagnostics, or missing/nil rules.

## Pinned upstream evidence

The inspected oracle is Luau 0.709, commit
`b968ef742741bb2b703afc3b3c53f06608c87481`, vendored by
`luau0-src 0.18.3+luau709`. Its source chain is:

- `VM/src/lstrlib.cpp`, `str_sub`: obtains start through `luaL_checkinteger`
  and end through `luaL_optinteger`.
- `VM/src/laux.cpp`, `luaL_checkinteger`: calls `lua_tointegerx`; the optional
  helper defaults only missing/nil values.
- `VM/src/lapi.cpp`, `lua_tointegerx`: converts the numeric value with
  `luai_num2int`; it does not check the representable integer range.
- `VM/src/lnumutils.h`, `luai_num2int`: expands to `((i) = (int)(d))`.

C++ floating-to-integral conversion truncates toward zero, but conversion is
undefined when the truncated result cannot be represented. NaN and infinities
also cannot produce a representable result. A local oracle observation outside
that domain is evidence about that build, not a portable language contract.
The pinned sources do not supply a deterministic fallback for that case.

Fresh oracle correctness probes confirmed these safe-domain substring results:

| Arguments after the string `abcdef` | Result |
|---|---|
| `2.9` | `bcdef` |
| `-2.9` | `ef` |
| `1, 3.9` | `abc` |
| `1, -2.9` | `abcde` |
| `-0.9` | `abcdef` |
| `1, -0.9` | empty string |
| `"2.9", "4.9"` | `bcd` |
| `2.9, nil` | `bcdef` |

Numeric string input `string.sub(12, 1.9)` returns `12`. These observations were
obtained through `error-oracle-worker luau`; no performance measurement was run.

## Regression coverage and implementation gate

`crates/rustia/tests/numeric_index_contract.rs` checks direct and aliased calls,
both index positions, NaN/infinities, finite fractions, numeric-string coercion,
zero, optional nil, and the exact signed-64 boundaries. It runs across
LuauCompat, ReproducibleStandard, DeterministicSim and Puc51Compat before and
after decision reset. Rejected inputs are caught through guest `pcall` and
followed by a successful evaluation.

The upper-bound regression exposed acceptance of `2^63`; the corrected
exclusive comparison now rejects it. Native tests pass; CI verifies the exact
candidate on the existing native and Node targets. No unchecked upstream result should be
added as a Luau-parity fixture. Broader integer conversions and supported-target
execution remain separate audit work; this scoped contract does not close the
public-release compatibility or platform matrix gates.

## Performance isolation

The first eight-leg acceptance measured generic iteration +2.14% and math/table
+1.53%, with a closure outlier. Its repeat included zero in every interval but
flagged a sparse-table outlier. The sixteen-leg comparison at `78b783c9`
([run](https://github.com/slowchop/rustia/actions/runs/40061724219)) measured
generic iteration +2.13% [1.68%, 2.94%] and math/table +1.85% [1.00%, 3.00%],
with another sparse-table outlier. The separate identical-binary control
([run](https://github.com/slowchop/rustia/actions/runs/40063636709)) failed its
straddling comparison: math/table read -1.737% [-3.18%, -0.80%] despite identical
code. These results do not establish clean acceptance. Thresholds are unchanged.

Neither workload calls the string index helper. An equivalent strict-greater-than
comparison against the last valid f64 is an isolation candidate for a possible
code-generation/layout effect, not a measured performance fix. The numeric
contract and its existing adjacent-boundary tests remain unchanged. Remote
comparison must establish whether this candidate resolves the movement.

The first isolation candidate did not improve performance: direct comparison
[40069181372](https://github.com/slowchop/rustia/actions/runs/40069181372)
found no change, while baseline comparison
[40069183742](https://github.com/slowchop/rustia/actions/runs/40069183742)
still measured iteration +2.21% [1.61%, 3.32%] and math/table +1.81%
[0.87%, 2.65%], without flagged outliers. This is not accepted.

Static Linux x86-64 compilation with Rust 1.95.0 found eight functions grew
and 1,538 symbol addresses/sizes changed versus the original baseline. This
shows a layout effect exists, not that it explains the measured slowdown.
The next isolation candidate keeps the shared checked conversion out of line
to separate its bounds/cast expansion from its callers. This may also impose
call overhead on string operations; remote measurements, including relevant
string workloads, are required before adopting it. No timing was collected
from the local disassembly builds, and thresholds remain unchanged.

The out-of-line experiment was rejected after
[40231066927](https://github.com/slowchop/rustia/actions/runs/40231066927):
against `4ca0b13b`, `5ddfdd76` measured dense tables +0.81% [0.05%, 1.68%],
sparse tables +2.33% [0.79%, 3.89%], closures +4.53% [2.50%, 5.78%],
iteration +1.36% [0.34%, 2.66%], and math/table -0.11% [-1.44%, 3.82%].
These are eight-leg, 99% intervals; no outlier was flagged. The no-inline
attribute is removed rather than retained as an unproven optimization.

Disassembly of the earlier inline candidate shows an explicit truncation call
and a separate finiteness check in each conversion. The next candidate uses
only the half-open signed-64 range check and Rust's truncating cast. NaN fails
range containment and infinities lie outside the range. Every representable
f64 near either signed-64 boundary is already integral (spacing at least 1024),
so checking the original value instead of its truncation accepts exactly the
same inputs. Fractional values, signed zero and subnormals retain truncation
toward zero. This removes redundant work but remains a performance candidate
until remote baseline comparison passes; no threshold or prior evidence changes.

The checked-cast candidate `f54b0774` also failed acceptance in
[40234600209](https://github.com/slowchop/rustia/actions/runs/40234600209).
The original baseline remained `90bc2ae4`; this run used sixteen legs per side,
ABBA ordering, five repetitions per leg and 99% intervals:

| Workload | Fixed N on both sides | Delta | 99% interval |
|---|---:|---:|---|
| Dense tables | 2333 | +2.69% | [2.02%, 3.17%] |
| Sparse tables | 1298 | +2.11% | [1.31%, 3.26%] |
| Closures/upvalues | 632142 | +2.56% | [0.13%, 4.13%] |
| Generic iteration | 8272 | +3.70% | [3.10%, 5.06%] |
| Math/table | 83511 | +0.68% | [-0.07%, 2.18%] |

Sparse-table baseline pass p4ab was 11.6% above its side's median leg, and
candidate pass p2ab was 11.1% above its side's median leg. Both remain included
in the intervals. The merged weekly matrix publication updated fixed loop counts
from 2317/1373/607654/8487/84037 to the values above; both revisions use the
same new counts. This is not an exact protocol repeat of the earlier comparisons.
No thresholds changed. The smaller conversion code and passing correctness
checks do not establish performance acceptance; this result is retained as a
failure, not rerun until green.

The follow-up unique-symbol audit found eight functions shrink in the checked-cast
binary; the named unique hot-loop instruction bodies were unchanged after
normalizing relocations. Address changes alone do not prove the regression's
cause. The acceptance workflow's explicit `profile` mode therefore collects
cycles, instructions, branch and cache counters in ABBA order on the same pinned
host, fixed harness and loop counts. It checks workload checksums and idle/steal
gates, retains raw counters, and produces no acceptance or publication artifact.
Counter totals include harness setup and warmup; they are diagnostic only.
Normal acceptance and committed calibration remain unchanged.

The first diagnostic dispatch
[40241006935](https://github.com/slowchop/rustia/actions/runs/40241006935)
failed the counter-access probe before compilation or measurement. The runner
host denied perf events. Diagnostics now request only user-space counters and
fetch failed exports before reporting failure; workflow artifacts retain those
exports for review. This is an infrastructure prerequisite, not a performance
result. The f54b0774 screening run
[40234604744](https://github.com/slowchop/rustia/actions/runs/40234604744)
also retained dense +2.36% [0.65%, 3.28%], sparse +0.93% [0.03%, 1.85%],
closures +2.25% [-3.29%, 6.35%], iteration +3.15% [2.85%, 4.00%], and
math/table +1.59% [-0.96%, 3.44%]; it does not overturn failed acceptance.

The authorized user-space counter run
[40242289162](https://github.com/slowchop/rustia/actions/runs/40242289162)
completed in ABBA order on the original baseline and checked-cast candidate.
Both sides used the same frozen workloads, loop counts and five repetitions.
Mean counter differences (two legs per side, diagnostic only) were:

| Workload | Cycles | Retired instructions | Branch misses | Cache references |
|---|---:|---:|---:|---:|
| Dense tables | +2.044% | +0.003% | +0.001% | +3.059% |
| Sparse tables | +1.543% | +0.019% | +0.470% | -6.472% |
| Closures/upvalues | +1.372% | approximately unchanged | +1.808% | -0.544% |
| Generic iteration | +4.906% | approximately unchanged | -2.776% | +24.838% |
| Math/table | +0.435% | approximately unchanged | -7.359% | +4.882% |

Counter totals include setup and warmup. These are not acceptance intervals.
The iteration movement is consistent with changed cache behavior, not added
executed instructions or increased branch misses. It does not isolate a specific
cache level or prove causation. Raw exports are retained by the workflow;
temporary counter access was restored after the run.

The next candidate keeps the checked numeric conversion and moves `Error::new`
out of line as cold code. The constructor had an explicit inline annotation and
hundreds of checked-access call sites, including the main dispatch function.
This targets duplicated error-payload stores in successful execution code rather
than adding padding or trying another equivalent numeric expression. Generated
code size, correctness, original-baseline acceptance and protected-call workload
cost must be checked before adoption. Error kinds, payloads and source-location
rules remain unchanged.

The generic-constructor outline was rejected by static inspection before any
remote timing. With the same pinned Rust 1.95.0 Linux build, the large dispatch
closure grew from 110,416 to 151,470 bytes and the outer closure from 37,370 to
41,491 bytes. Passing the large error enum out of line materialized copies that
inlining had eliminated. The generic constructor's original inline annotation
is restored; this experiment supplies no performance measurement.

The replacement is narrower: `Error::invalid_program` takes only the static
reason string, constructs that specific error out of line, and serves the 59
invalid-bytecode checks within `run_function_step`. It keeps ordinary guest
error construction unchanged, including protected-call error handling. This
preserves error payloads and location handling while targeting cold internal
validation code. Static size reduction and remote acceptance are still gates;
no source-level claim establishes a speedup.

The narrow helper also failed the static footprint criterion: the large dispatch
closure grew to 113,604 bytes and the outer closure to 39,379 bytes. It was
removed before timing. The next static candidate retains inline construction
and adds the cold annotation only, preserving specialization while giving the
compiler branch-frequency information. This is still an experiment, not an
accepted performance improvement.

The cold-inline annotation also failed the static footprint check (dispatch
closures 38,483 and 111,940 bytes), and is removed. No cold-constructor variant
was timed. The replacement extracts the nine string-opcode bodies into one
non-inlined dispatch helper, preserving every register access, fuel charge and
error-PC attachment in order. This tests a concrete reduction in the main
dispatch footprint rather than another numeric expression or error annotation.
Both original-baseline acceptance and string-workload cost must pass remotely
before adoption; static size alone is not acceptance.

The extracted candidate `c1913422` passes the static footprint gate: the large
dispatch closure shrinks from 110,416 to 106,454 bytes (3,962 bytes), while the
outer closure stays at 37,370 bytes. The new helper is 10,415 bytes; total binary
size reduction is not claimed. Default/bare numeric-contract, diagnostic-parity
and error-recovery tests pass. Remote five-workload and supplemental string
comparisons remain required. Delivery head `02c0b174` was pushed after full
pre-push validation. Original-baseline sixteen-leg comparison
[40246582277](https://github.com/slowchop/rustia/actions/runs/40246582277)
and supplemental string comparison
[40246584665](https://github.com/slowchop/rustia/actions/runs/40246584665)
were dispatched; their completed intervals must be inspected before acceptance.

The supplemental `strings` workflow mode freezes the existing paired harness
and measures a dedicated `string_opcodes` workload at N=10,000, five repetitions
and eight ABBA legs per side. Nested function compilation intentionally disables builtin fast paths, so this
supplement uses a top-level program with its loop count supplied as a global,
rather than the normal returned-function adapter. A correctness-only compilation
test requires all nine specialized string opcodes in that workload. The run retains source hashes,
raw samples and host gates, and requires identical checksums across both
revisions and all legs. Its 99% intervals are reported without inventing a
calibrated string materiality threshold. Normal five-workload acceptance and
the committed calibration are unchanged.

The string-dispatch candidate failed original-baseline acceptance in
[40246582277](https://github.com/slowchop/rustia/actions/runs/40246582277),
although the workflow completed successfully. Sixteen ABBA legs per side and
five repetitions produced these 99% intervals:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -0.57% | [-1.26%, 0.97%] |
| Sparse tables | +0.49% | [-0.52%, 1.00%] |
| Closures/upvalues | +2.98% | [1.74%, 5.22%] |
| Generic iteration | +0.33% | [-0.04%, 1.25%] |
| Math/table | -0.56% | [-1.29%, 0.60%] |

No leg was flagged by the existing outlier rule. Every paired closure median
was slower on the candidate. The resolved iteration and math/table intervals
do not justify accepting the closure regression. All 192 raw sidecars across
16 pairs were retained locally, alongside the complete job log. No thresholds
changed, and this result will not be rerun until green.

Static inspection found that LLVM already combines the two selected-capture
lookups into one function-record access; a proposed capture-slice API was
therefore rejected without implementation. The next candidate instead removes
an observed full caller-register scan after successful scalar leaf calls when
loader analysis of actual bytecode proves that caller cannot create local
capture cells. Any local-cell read/write or function creation conservatively
keeps synchronization. This proof is independent of optional capture hints;
unknown frames keep the old scan. Correctness and fresh baseline comparison
remain required, with all preceding results retained.

The completed supplemental string comparison
[40246584665](https://github.com/slowchop/rustia/actions/runs/40246584665)
measured `02c0b174` against original baseline `90bc2ae4`: -1.57%, with a
99% interval of [-2.46%, -0.41%]. All sixteen raw legs use fixed N=10,000,
five repetitions, and matching checksums. The frozen source hash is
`59367a09e7a701f22784cdd78ea5f4b3d322fb601f2a31edbfe5b0e8d4ffa847`.
This supports the extraction's string cost but does not resolve the separate
closure regression above; no string materiality threshold was invented.

The caller-cell proof is implemented at `aa432f17`; delivery head `4be30aca`
passed the full pre-push inventory before remote validation. Fresh
[sixteen-leg original-baseline acceptance](https://github.com/slowchop/rustia/actions/runs/40253017402),
[eight-leg string acceptance](https://github.com/slowchop/rustia/actions/runs/40253020172),
and [manual platform CI](https://github.com/slowchop/rustia/actions/runs/40253022712)
were dispatched for that candidate. Their completed evidence remains a gate;
neither the earlier string result nor passing correctness tests establishes
performance acceptance for the new runtime.


The caller-cell candidate failed its completed sixteen-leg comparison
[40253017402](https://github.com/slowchop/rustia/actions/runs/40253017402).
All 192 raw sidecars and the complete workflow log are retained locally. Five
repetitions per leg and unchanged 99% analysis produced:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +1.14% | [0.69%, 1.81%] |
| Sparse tables | +0.24% | [-0.65%, 1.46%] |
| Closures/upvalues | -1.04% | [-2.01%, 0.07%] |
| Generic iteration | +4.00% | [2.81%, 5.28%] |
| Math/table | +1.32% | [0.00%, 3.38%] |

The math/table lower bound rounds to zero but is positive in the unrounded
analysis. Sparse baseline pass p9ba is 5.3% above its side's median and remains
included. Resolving the closure interval does not justify the other slowdowns;
thresholds remain unchanged and this result is not accepted.

Inspection found an avoidable cost introduced by the caller-cell candidate:
it looked up the caller proof on every function-frame entry, including frames
that never execute a successful scalar leaf call. The correction reads the
proof from the same function cache already consulted by an eligible scalar
call and returns it alongside a successful outcome. Cache misses retain the
caller's proof; unknown caller identities conservatively keep synchronization.
This removes the added per-frame lookup without weakening the proof or
changing fuel, mutation, GC guards or numeric conversion. Fresh remote
acceptance is required; this inspection is not a performance claim.


The retained caller-cell candidate's supplemental string comparison
[40253020172](https://github.com/slowchop/rustia/actions/runs/40253020172)
completed after its failed five-workload comparison. Against original baseline
`90bc2ae4`, delivery head `4be30aca` measured -1.43%, with a 99% interval of
[-2.67%, 0.12%]. All sixteen individual raw legs and their aggregate copies
validate the frozen source hash, full commit identities, N=10,000, five
repetitions, matching checksums and ABBA timestamp order. Independent raw-sample
analysis exactly reproduces the reported interval. The largest absolute leg
median deviation is 1.63%; no string materiality threshold is introduced.
This historical string result does not overturn the candidate's failed
five-workload acceptance or establish acceptance for the corrected lookup.


The lookup correction `6fd0e482`, delivered as `6b0122bd`, failed complete
[sixteen-leg acceptance](https://github.com/slowchop/rustia/actions/runs/40259998147)
despite the workflow completing successfully. Independently recomputed 99%
intervals from all 192 retained sidecars are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -3.38% | [-4.60%, -2.86%] |
| Sparse tables | +2.02% | [0.58%, 3.60%] |
| Closures/upvalues | -5.34% | [-6.38%, -3.15%] |
| Generic iteration | -1.21% | [-1.89%, -0.43%] |
| Math/table | +1.66% | [0.78%, 2.26%] |

All 160 individual sidecars and 32 aggregate copies validate full identities,
fixed N, five repetitions, checksums and ABBA timestamp order. Sparse baseline
pass p10ab is 5.30% above its side median and candidate p12ab is 8.60% above;
both remain included. Math/table has no flagged outlier. The two positive
intervals remain unresolved; material-regression verdicts are inconclusive,
not exclusions. Thresholds and all earlier evidence are unchanged.

Inspection identified two redundant operations in the affected paths. Every
Log-table write did a preliminary hash lookup whose result was used only to
decide whether a vector key needs an owned visible copy; non-vector keys now
skip that preliminary lookup. Ordinary unary math calls copied already
validated arguments into a temporary vector before a shared helper only read
them; they now borrow the original slice. Existing validation, numeric coercion,
libm operations, key normalization, vector behavior, quota staging, journaling
and mutation accounting remain unchanged. These concrete work reductions are
candidates, not performance claims, until fresh remote acceptance succeeds.


The preceding lookup candidate's supplemental string run
[40260011328](https://github.com/slowchop/rustia/actions/runs/40260011328)
measured `6b0122bd` at +0.45% [-0.37%, 1.29%], a 99% interval spanning zero.
Independent analysis validates all sixteen raw legs and aggregate copies,
frozen source hash, full identities, N=10,000, five repetitions, matching
checksums and precise ABBA order. The largest absolute leg deviation is 1.05%,
with no apparent outlier. The raw export and full log are retained. This does
not resolve that candidate's sparse-table and math/table regressions above.

The table/math correction `550d9b1b`, delivered as `8c96cf24`, failed
[sixteen-leg original-baseline acceptance](https://github.com/slowchop/rustia/actions/runs/40282644301).
All 192 raw sidecars are retained and independently validated for full identities,
fixed N, repetitions, checksums, aggregate copies and ABBA timestamp order.
The unchanged 99% analysis gives:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +1.86% | [0.75%, 2.98%] |
| Sparse tables | +1.04% | [0.19%, 2.93%] |
| Closures/upvalues | +1.55% | [-0.03%, 3.06%] |
| Generic iteration | +5.89% | [5.09%, 6.49%] |
| Math/table | -4.00% | [-4.65%, -2.98%] |

Candidate pass p11ba sparse and math/table medians are respectively 6.84% and
7.15% above their side medians; both flagged outliers remain included. Improved
math/table does not resolve the other positive intervals. No thresholds changed.

The next correction removes an unconditional register-cell synchronization scan
on fresh function frames, whose cell vector has just been initialized entirely
to None. Resumed frames still synchronize saved registers before applying child
results and rebuilding stackless roots. This removes concrete redundant work
from fresh calls without changing cell aliases, fuel or collection boundaries;
fresh remote evidence is required before any performance claim.

The preceding table/math candidate's supplemental string comparison
[40282653930](https://github.com/slowchop/rustia/actions/runs/40282653930)
measured `8c96cf24` at +0.21% [-1.43%, 1.39%]. Independent analysis validates
all sixteen raw legs and aggregate copies, frozen source hash, full identities,
N=10,000, five repetitions, matching checksums and ABBA order. The largest
absolute leg deviation is 1.76%, with no apparent outlier. This interval spans
zero but does not overturn the failed five-workload acceptance above.

The fresh-frame correction is runtime `3b776b31`, delivered as `4846cd1e`
after full pre-push validation. Its
[original-baseline comparison](https://github.com/slowchop/rustia/actions/runs/40291359314)
and [string comparison](https://github.com/slowchop/rustia/actions/runs/40291363030)
were dispatched for that exact head. The completed baseline failure is recorded
below; the completed string result follows it. The code change itself is not
performance evidence.

The fresh-frame candidate `4846cd1e` failed complete
[sixteen-leg acceptance](https://github.com/slowchop/rustia/actions/runs/40291359314).
All 192 sidecars are retained and independently validate identities, fixed N,
five repetitions, checksums, aggregate copies and all 80 ABBA workload pairs.
No outliers were flagged. Recomputed 99% intervals are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +1.85% | [0.51%, 2.47%] |
| Sparse tables | +0.98% | [-0.06%, 2.31%] |
| Closures/upvalues | +2.25% | [0.48%, 3.40%] |
| Generic iteration | +5.50% | [4.83%, 6.56%] |
| Math/table | -4.44% | [-5.19%, -3.72%] |

The new scan removal did not resolve the positive dense/closure/iteration
intervals. Both `8c96cf24` and `4846cd1e` reverse the earlier caller-proof
candidate's direction on those workloads, while improving math/table. The next
isolation therefore reverts the table-probe and fresh-frame scan changes and
retains only borrowed unary math arguments relative to `6b0122bd`. All tests,
failed comparisons and thresholds remain intact. A direct eight-pair comparison
against `6b0122bd` will isolate that math change; it cannot substitute for final
original-baseline acceptance or establish code-layout causality by itself.

The fresh-frame candidate's completed supplemental string comparison
[40291363030](https://github.com/slowchop/rustia/actions/runs/40291363030)
measures `4846cd1e` at -0.18% [-1.39%, 0.92%], a 99% interval spanning zero.
The retained export validates all sixteen raw legs and aggregate copies, full
identities, frozen source hash, fixed N=10,000, five repetitions, checksums and
ABBA order. This does not resolve its failed five-workload acceptance.


The unary-math-only candidate `ecbd774d`, delivered as `6b06808b`, fails its
[direct eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40305656584)
against `6b0122bd`. All 96 sidecars independently validate full identities,
fixed N, five repetitions, checksums, sixteen aggregate copies and forty ABBA
workload pairs. Recomputed 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +4.27% | [3.16%, 6.59%] |
| Sparse tables | -0.26% | [-3.75%, 1.28%] |
| Closures/upvalues | +8.13% | [6.28%, 9.97%] |
| Generic iteration | +7.38% | [2.48%, 8.59%] |
| Math/table | -5.66% | [-6.58%, -4.69%] |

Sparse baseline pass p4ab is 6.87% above its side median and remains included.
Dense tables and closures exceed the unchanged material thresholds; iteration
is directionally slower with an inconclusive material verdict. The green
workflow means measurement completed, not acceptance. Original-baseline
acceptance is not dispatched for this failed candidate.

Static Linux x86-64 builds, never executed for timing, show shared native-call
dispatch growing from 8,016 to 8,148 bytes. Main interpreter function sizes
remain unchanged but addresses shift sixteen bytes. The largest inner dispatch
has 19,793 identical normalized instruction lines after relocation normalization.
The remote isolation establishes that this code change shifts unrelated workload
timings; these static observations do not prove code-layout causality.


The next bounded experiment marks only the existing `native_math_unary`
wrapper `inline(never)`, retaining borrowed arguments and unchanged semantics.
A static build reduces shared native-call dispatch from 8,148 to 8,092 bytes
and emits a 98-byte wrapper; the shared math implementation remains 742 bytes.
This confirms the intended separation, not a speedup: the additional call
boundary may offset allocation savings on cheap math operations. A fresh
remote eight-pair comparison against `6b0122bd` must test the combined isolated
math change before any original-baseline acceptance is warranted.


The wrapper candidate `2e347dad`, delivered as `457d7715`, completes its
[eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40311566683)
against `6b0122bd` but does not resolve acceptance. All 96 sidecars independently
validate identities, fixed work, repetitions, checksums, aggregate copies and
forty ABBA workload pairs. No outliers were flagged. The 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -0.61% | [-2.94%, 1.69%] |
| Sparse tables | +1.56% | [-0.25%, 2.49%] |
| Closures/upvalues | -0.65% | [-3.42%, 1.41%] |
| Generic iteration | +5.28% | [2.38%, 6.18%] |
| Math/table | -5.27% | [-7.20%, -3.09%] |

The wrapper removes the previous dense/closure directional regressions, but
iteration remains slower. Excluding its calibrated 7.60% material threshold
does not satisfy the requirement to resolve persistent directional slowdowns.
Sparse-table material acceptance is inconclusive. No original-baseline run is
dispatched for this candidate; all evidence and thresholds remain intact.


The failed wrapper annotation is removed. The next bounded candidate separates
unary math's value operation from opcode entry validation. Native calls validate
the argument count, then pass the first value directly; opcode calls validate
arity and register access before the same value coercion and libm operation.
This removes the native temporary-register copy and synthetic instruction,
without removing either entry's validation or changing error ordering.
Static Linux x86-64 inspection gives a 147-byte opcode entry and a 463-byte
shared value operation, versus the prior 742-byte opcode-oriented helper.
Native dispatch is 8,141 bytes; the main interpreter body remains 106,297 bytes.
These sizes are diagnostic only. Fresh remote isolation, then original-baseline
and string acceptance if justified, remain required.

Targeted regressions check direct and aliased unary calls across all four
profiles for numeric-string coercion, evaluated-but-ignored extras and
missing/invalid values. A VM entry test checks native argument-count bounds
and opcode arity-before-register validation. Both feature modes pass these
checks and the numeric-index and suspended-call contracts.


The shared-value candidate `12d16851`, delivered as `6a8b61b1`, completes its
[eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40317807249)
against `6b0122bd` without establishing acceptance. All 96 sidecars independently
validate full identities, fixed N, repetitions, checksums, aggregate copies and
forty ABBA workload pairs. The 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +0.01% | [-0.71%, 2.03%] |
| Sparse tables | +0.68% | [-2.39%, 1.75%] |
| Closures/upvalues | +4.51% | [3.09%, 12.20%] |
| Generic iteration | +3.42% | [2.60%, 6.92%] |
| Math/table | -6.77% | [-8.43%, -4.34%] |

Candidate closure pass p7ba is 12.09% above its side median and remains included.
Iteration and closures remain directionally slower. No thresholds or samples
are changed, and no original-baseline acceptance is dispatched for this result.

The shared math operation is retained provisionally for its concrete work
reduction and measured math improvement, not as an accepted overall candidate.
The next correction addresses concrete iteration work: the recognized
`ArrayIterator` leaf reuses the existing exact-positive-i32 array-slot reader
before general numeric table lookup. Present nil slots remain successful reads;
all other keys or missing array slots use the original lookup. Metatable and
length guards still precede mutation; result counts, instruction charges and
collection/fuel admission are unchanged. Forced-leaf versus scalar tests cover
array values, holes, exhaustion, fractional/negative/zero keys and a verified
PUC hash-backed numeric slot across the applicable profiles. A direct eight-pair
comparison against `6a8b61b1` will isolate this lookup change. Final acceptance
still requires the original `90bc2ae4` baseline and the string workload.


The numeric-slot candidate `cd1071cf`, delivered as `2eaf304e`, fails its
[eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40324770013)
against `6a8b61b1`. All 96 raw sidecars, full identities, fixed N, repetitions,
checksums, aggregate copies and forty ABBA workload pairs independently validate.
No outliers were flagged. The 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | +1.52% | [-0.65%, 2.77%] |
| Sparse tables | -0.95% | [-2.30%, 0.15%] |
| Closures/upvalues | +4.68% | [3.30%, 6.44%] |
| Generic iteration | -0.56% | [-3.69%, 0.40%] |
| Math/table | -0.82% | [-2.59%, 0.68%] |

The closure slowdown is statistically clear; iteration does not establish a
win. The production slot-read change is reverted, retaining its semantic
regression. No samples or thresholds are changed and no final original-baseline
acceptance is dispatched for this failed candidate. Inspection of the prior
pinned Linux binary shows the two selected capture reads already share one
function-arena lookup, so a pair accessor alone is not justified by source-level
repetition. The separate function-kind lookup remains under investigation.


The next isolated correction borrows function kind and captures from one
validated arena slot during scalar leaf admission. Only the selected capture
pair is copied before cache mutation. Existing admission checks, cache counts,
missing-capture fallback and guard-before-upvalue-write behavior remain intact.
This targets the separately emitted function-kind and selected-capture lookups,
not the already-coalesced reads within capture selection. Direct isolation uses
`6a8b61b1`, with the failed slot read reverted, so its only production difference
is this metadata access. Original-baseline and string acceptance remain required.


Static inspection rejected a generic kind-and-captures tuple because it loaded
capture metadata before rejecting native calls. The narrowed accessor rejects
non-retained-bytecode kinds first. Its pinned Linux output removes the scalar
path's second validated function lookup, but introduces a 119-byte helper call
and grows the main inner dispatch from 106,297 to 106,805 bytes. These are
explicit code-shape tradeoffs, not speed claims. Remote isolation must establish
whether the removed lookup offsets those costs; no inline-annotation sweep or
threshold change is used. Default and bare VM suites each pass 249 tests,
including incomplete captures on a warmed cache with unchanged mutation and
diagnostic counts.


The retained-bytecode metadata candidate `12521d60`, delivered as `7cbdd866`,
completes [eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40332718288)
against `6a8b61b1`. Independent audits validate all 96 sidecars, full identities,
fixed N, repetitions, checksums, aggregate copies and forty ABBA workload pairs.
No outliers were flagged. The 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -0.79% | [-2.61%, 0.83%] |
| Sparse tables | -1.89% | [-4.17%, -0.07%] |
| Closures/upvalues | -5.81% | [-7.37%, -3.69%] |
| Generic iteration | +3.06% | [1.72%, 4.31%] |
| Math/table | +0.28% | [-1.84%, 2.76%] |

Overall acceptance fails: the closure win does not excuse the directional
iteration slowdown, and math/table's material verdict is inconclusive. The
metadata change is provisionally retained for its demonstrated closure benefit,
subject to the next compound gate, not marked accepted. No thresholds or samples
change and no final original-baseline comparison is dispatched yet.

The generic iterator route separately resolves the function kind to reject
built-in `next`/`inext`, then resolves it again in ordinary call dispatch.
Pinned Linux disassembly confirms both validated arena reads. The next correction
reuses the already-resolved handle/kind through the existing shared executors
with iterator-specific entry glue. Resolution
remains after iterator-callability and state/control checks; built-in fuel and
recursion handling stay unchanged. Valid non-built-in functions check recursion
before shared leaf/fallback dispatch; invalid handles and nonfunctions use the
original path to preserve error precedence and metamethod behavior. No guest
execution or heap mutation occurs between resolution and reuse. Direct eight-pair
isolation uses `7cbdd866`; original-baseline and strings remain mandatory.


The first refactor extracted a shared resolved-call helper, but pinned static
inspection showed an additional call boundary on unrelated ordinary calls.
That form was rejected before timing. The scoped form restores ordinary
call-value dispatch unchanged and reuses existing leaf and general executors
only from the iterator entry. Its iterator function grows from 3,462 to 4,950
bytes; ordinary call-value remains 1,356 bytes, leaf entry 611 bytes, and main
inner dispatch 106,805 bytes. This localized size increase is an explicit
tradeoff, not performance evidence. Independent review found no semantic issue.
Default and bare VM suites each pass 250 tests, including error precedence.
The coroutine regression covers direct and callable-table iterators, yields,
deferred calls and synchronization of captured caller state across four profiles.


The scoped iterator-resolution runtime `e292ef2b`, delivered as `c754371f`,
completes [eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40341946265)
against `7cbdd866`. All 96 raw sidecars and forty ABBA workload pairs validate,
with no outliers. Dense tables measure +0.65% [0.05%, 1.77%], sparse tables
+0.16% [-1.47%, 1.33%], closures +0.78% [-1.69%, 2.67%], iteration
-1.59% [-5.97%, 0.03%], and math/table -2.45% [-4.38%, -0.41%].
Iteration's improvement is indistinguishable at 99%; this is not acceptance.

The direct [sixteen-pair original-baseline comparison](https://github.com/slowchop/rustia/actions/runs/40347110048)
against `90bc2ae4` avoids combining deltas from separate runs. All 192 sidecars,
full identities, fixed N, repetitions, checksums, aggregate copies and eighty
ABBA workload pairs independently validate. Its 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -2.69% | [-3.18%, -1.79%] |
| Sparse tables | +0.97% | [0.14%, 1.73%] |
| Closures/upvalues | -6.44% | [-7.41%, -5.88%] |
| Generic iteration | +2.59% | [1.59%, 3.30%] |
| Math/table | -7.09% | [-9.85%, -5.69%] |

Acceptance fails the persistent sparse-table and iteration slowdowns. The sparse
material verdict is inconclusive. Baseline pass 6 outliers, sparse +7.816803%
and math/table +6.201187%, remain included; no samples or thresholds change.

Static inspection confirms an unconditional non-nil hash vacancy lookup inside
ordinary `table_set`, before its vector-key decision and subsequent store. Only
vector keys need that vacancy result. The next isolated change gates that probe
by vector kind, preserving normalization, short-string interning, readonly checks,
vector replacement/nil behavior, PUC routing and quota-specific writes. The same
expression appeared in the earlier bundled `550d9b1b` experiment, but that run
also changed unary math and never isolated this probe. The new direct comparison
uses `c754371f` and changes only this guard; it does not repeat an unchanged run.
Iteration remains separately unresolved. No broader capture-admission refactor
or annotation sweep is bundled into this correction.


The same `c754371f` candidate's [eight-pair string comparison](https://github.com/slowchop/rustia/actions/runs/40347116567)
measures -3.17% [-4.09%, -2.07%]. All sixteen individual legs and sixteen
aggregate copies validate against the pinned source hash, N=10,000, five
repetitions, checksums and ABBA timestamps. This improvement does not resolve the
sparse-table or iteration blockers.

For the isolated vector-probe guard, pinned Linux static output moves the vector
check before the vacancy lookup; non-vector writes skip that call. `table_set`
grows from 3,348 to 3,397 bytes. Main dispatch keeps its 106,805-byte size and
starting address. These are code-shape observations, not performance claims.
Default and bare VM suites each pass 250 tests; focused numeric-index, array
mutation and golden facade checks pass eight tests total per feature mode.


The isolated vector-probe runtime `8a1c6ac4`, delivered as `f1e6de36`, completes
[eight-pair comparison](https://github.com/slowchop/rustia/actions/runs/40361503127)
against `c754371f`. All 96 raw sidecars and forty ABBA workload pairs validate;
no outliers are flagged. At 99% confidence, dense tables measure +0.32%
[-0.62%, 0.77%], sparse tables -1.34% [-2.84%, 0.58%], closures +0.21%
[-1.40%, 1.45%], iteration +0.51% [0.11%, 0.97%], and math/table +0.25%
[-1.49%, 1.37%]. Sparse improvement is unestablished and iteration slows.
The production probe guard is reverted rather than accumulated; no samples or
thresholds change, and no original-baseline acceptance is run for this candidate.

The next correction targets a separate emitted duplicate read in the iterator
leaf: its metatable check validates the table arena slot, then `array_len`
validates that same slot again. A crate-private primitive-boundary query performs
one validation, rejects any metatable, and calls the existing boundary function.
It preserves Log/PUC boundaries and guard-before-index-mutation ordering; general
length and numeric lookup remain unchanged. The direct baseline is `c754371f`,
with the failed probe reverted, so only this iterator guard differs in production.
The four-profile regression explicitly covers metatable and missing-table
fallback without mutation, followed by normal results after metatable removal.


Pinned Linux static output inlines the combined guard and calls the existing
boundary function directly, eliminating the second arena validation. The leaf
outcome executor shrinks from 1,045 to 992 bytes; main dispatch retains its
106,805-byte size and starting address. No new wrapper call remains. Default
and bare VM suites each pass 251 tests; focused continuation, numeric-index,
metamethod-yield and golden checks pass nine tests total per feature mode.
These correctness and code-shape checks do not establish a speed improvement.


The combined-boundary runtime `dc2adfd2`, delivered as `7b450eb3`, completes
[eight-pair isolation](https://github.com/slowchop/rustia/actions/runs/40369557729)
against `c754371f`. Both audits validate all 96 sidecars, full identities,
fixed work, repetitions, checksums, aggregate copies and forty ABBA workload
pairs. No outliers are flagged. The 99% results are:

| Workload | Delta | 99% interval |
|---|---:|---|
| Dense tables | -0.10% | [-1.20%, 0.78%] |
| Sparse tables | -0.44% | [-1.04%, 0.90%] |
| Closures/upvalues | +0.87% | [-1.00%, 4.04%] |
| Generic iteration | -0.13% | [-0.90%, 3.25%] |
| Math/table | -0.08% | [-1.69%, 2.70%] |

No workload establishes improvement. Closure and math/table material verdicts
remain inconclusive. The production experiment is reverted while retaining its
four-profile guard regression and all evidence. No original-baseline acceptance
is run for this unproven candidate and no thresholds or samples change.

A fresh [remote counter diagnostic](https://github.com/slowchop/rustia/actions/runs/40374618824)
compares retained `c754371f` with original `90bc2ae4`. It uses the existing fixed
five-workload profile and four ABBA legs. Counters include startup, compilation
and warmup; instrumented elapsed times are not acceptance evidence. Results
remain pending. Further corrections must follow this diagnostic rather than
accumulating unrelated small runtime rewrites.
