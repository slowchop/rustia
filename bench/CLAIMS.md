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
- Measurement window: 2026-09-15 02:48:00 UTC to 2026-09-15 02:52:23 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 185 | 5 / 10 | mlua-lua54 | 4.468x | 3.62x | 0.535x | not public-ready: rustia trails mlua-luau by 3.62x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 50,969 | 5 / 10 | wasmi | 18.64x | 2.655x | 0.194x | not public-ready: rustia trails mlua-luau by 2.655x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,235 | 8 / 10 | wasmi | 42.48x | 3.015x | 1.783x | not public-ready: rustia trails mlua-luau by 3.015x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 87,825 | 5 / 10 | wasmi | 37.17x | 4.012x | 0.604x | not public-ready: rustia trails mlua-luau by 4.012x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,195 | 6 / 10 | wasmi | 44.6x | 5.062x | 0.771x | not public-ready: rustia trails mlua-luau by 5.062x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.61x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.125x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.581x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.132x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.928x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.783x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.612x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.316x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.152x | 0 / 5 | linux-x86_64 | 5 workloads |
