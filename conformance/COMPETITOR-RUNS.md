# External implementation compatibility runs

This file records exploratory runs of rustia's fixture corpus against external
Luau implementations. These runs are diagnostic evidence only: they do not
certify conformance, measure performance, or define a public compatibility
ranking.

## 2026-07-22: luaur and ruau

The run used the crates.io releases `luaur` `0.1.8` and `ruau` `0.3.0`, each
through its default public Rust embedding API. The adapter ran every case in an
isolated process with a 20-second limit. It preserved the fixture source name
and supplied only the small `concat` helper used by two reduced fixtures.

The manifest then contained 692 executable cases:

- 643 reduced, self-contained fixtures under `conformance/fixtures/`;
- 49 whole upstream scripts under `conformance/upstream/`.

Only the reduced fixtures are comparable here. The whole scripts require
Luau's test-specific C++ harness and its native helpers, host bindings, and
resource policy. Counting their failures as VM compatibility failures would
not be meaningful.

| engine | reduced fixtures | crashes | timeouts |
|---|---:|---:|---:|
| rustia | 643 / 643 | 0 | 0 |
| luaur | 617 / 643 (96.0%) | 0 | 0 |
| ruau | 590 / 643 (91.8%) | 0 | 0 |

The raw result intentionally includes cases whose outcome depends on the host
or on diagnostic wording. For a more useful semantic comparison, 75 cases were
reported separately, leaving a 568-case portable-core subset:

| engine | portable core | excluded cases |
|---|---:|---:|
| rustia | 568 / 568 | 75 |
| luaur | 561 / 568 (98.8%) | 75 |
| ruau | 553 / 568 (97.4%) | 75 |

The excluded set comprises 34 exact-error checks, 11 OOM/resource-policy
checks, 9 GC-policy checks, 9 environment-harness checks, 7 userdata-surface
checks, 2 native-harness checks, 2 fixture-helper checks, and 1 entry-mode
check. Exclusion does not imply a passing result; it means the result should
not be folded into a portable VM-semantics percentage. In particular, both
engines accept the `\\q` escape where the Luau 0.709 fixture expects a syntax
error, and ruau's `table.unpack` large-result case returns instead of raising
the expected error.

The seven portable-core differences shared by both implementations cover
signed-zero behavior (`%`, `math.modf`, and table keys), `string.find` anchor
behavior, empty `string.gmatch` matches, overwritten generic-for iterators,
and legacy `table.foreach`/`table.foreachi` behavior. Ruau has eight additional
portable-core differences involving special `tonumber` strings, a plain
`gmatch` iterator, `table.create` coercion, table length/search boundaries, and
trailing multi-return expansion in a method call.

Interpretation:

- `luaur` is a useful close compatibility peer for targeted differential work.
  Its project describes itself as a faithful Luau port, but also distinguishes
  the low-level engine from the higher-level runtime surface in its
  [README](https://github.com/pjankiewicz/luaur) and
  [conformance notes](https://github.com/pjankiewicz/luaur/blob/main/docs/CONFORMANCE.md).
- `ruau` is a useful secondary boundary check, not a like-for-like replacement:
  its [README](https://github.com/cortesi/ruau) describes it as experimental and
  lists intentionally unsupported Luau features, including generic userdata.
- Keep the raw and portable-core denominators together whenever citing these
  results. A single percentage would hide important host, version, and surface
  differences. Do not use this report as a performance benchmark or public
  leaderboard.

The one-off runner and its tab-separated detail report were kept outside the
repository at the time of the run. Before turning this into a recurring CI job,
check in a maintained external-engine adapter with pinned source revisions and
explicit host-policy controls.
