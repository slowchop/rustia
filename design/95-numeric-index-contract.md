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
