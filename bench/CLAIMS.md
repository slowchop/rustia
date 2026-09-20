# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 65
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-09-20 20:32:29 UTC to 2026-09-20 20:36:53 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 177 | 5 / 10 | mlua-lua54 | 4.307x | 3.478x | 0.503x | not public-ready: rustia trails mlua-luau by 3.478x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,461 | 5 / 10 | wasmi | 18.75x | 2.734x | 0.197x | not public-ready: rustia trails mlua-luau by 2.734x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 12,988 | 8 / 10 | wasmi | 42.2x | 3.205x | 1.753x | not public-ready: rustia trails mlua-luau by 3.205x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 86,715 | 5 / 10 | wasmi | 36.52x | 3.87x | 0.597x | not public-ready: rustia trails mlua-luau by 3.87x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,174 | 6 / 10 | wasmi | 43.82x | 4.77x | 0.781x | not public-ready: rustia trails mlua-luau by 4.77x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.38x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.989x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.549x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.018x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.92x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.765x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.605x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.301x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.153x | 0 / 5 | linux-x86_64 | 5 workloads |
