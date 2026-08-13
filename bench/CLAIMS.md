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
- Measurement window: 2026-08-13 21:52:48 UTC to 2026-08-13 21:57:33 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 216 | 5 / 10 | mlua-lua54 | 4.634x | 3.355x | 0.643x | not public-ready: rustia trails mlua-luau by 3.355x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 52,145 | 5 / 10 | wasmi | 18.24x | 2.517x | 0.173x | not public-ready: rustia trails mlua-luau by 2.517x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,061 | 8 / 10 | wasmi | 35.52x | 2.789x | 1.892x | not public-ready: rustia trails mlua-luau by 2.789x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 90,872 | 5 / 10 | wasmi | 27.18x | 3.925x | 0.609x | not public-ready: rustia trails mlua-luau by 3.925x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,247 | 7 / 10 | wasmi | 48.55x | 4.979x | 0.728x | not public-ready: rustia trails mlua-luau by 4.979x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 15.81x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.769x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.409x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.169x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.953x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.775x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.622x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.311x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.158x | 0 / 5 | linux-x86_64 | 5 workloads |
