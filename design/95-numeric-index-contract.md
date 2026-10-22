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
upper test is `>= 2^63`; `-2^63` remains representable and valid. The adjacent
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
