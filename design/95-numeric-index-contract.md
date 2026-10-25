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
the implementation uses the latter to retain the original comparison form.
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
