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
- Measurement window: 2026-08-30 22:43:48 UTC to 2026-08-30 22:48:11 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 158 | 5 / 10 | mlua-lua54 | 3.816x | 3.093x | 0.454x | not public-ready: rustia trails mlua-luau by 3.093x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 42,861 | 4 / 10 | wasmi | 15.55x | 2.272x | 0.163x | not public-ready: rustia trails mlua-luau by 2.272x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 11,940 | 8 / 10 | wasmi | 38.84x | 2.94x | 1.593x | not public-ready: rustia trails mlua-luau by 2.94x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 82,929 | 5 / 10 | wasmi | 33.46x | 3.707x | 0.573x | not public-ready: rustia trails mlua-luau by 3.707x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,118 | 6 / 10 | wasmi | 41.75x | 4.703x | 0.742x | not public-ready: rustia trails mlua-luau by 4.703x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 15.54x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.652x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.246x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 2.777x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.833x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.694x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.549x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.274x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.14x | 0 / 5 | linux-x86_64 | 5 workloads |
