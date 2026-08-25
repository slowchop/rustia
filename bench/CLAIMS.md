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
- Measurement window: 2026-08-25 09:06:21 UTC to 2026-08-25 09:11:07 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 185 | 5 / 10 | mlua-lua54 | 3.821x | 2.988x | 0.439x | not public-ready: rustia trails mlua-luau by 2.988x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 50,621 | 4 / 10 | wasmi | 15.84x | 2.238x | 0.164x | not public-ready: rustia trails mlua-luau by 2.238x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,008 | 8 / 10 | wasmi | 35.75x | 2.543x | 1.487x | not public-ready: rustia trails mlua-luau by 2.543x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 97,377 | 5 / 10 | wasmi | 34.35x | 3.829x | 0.573x | not public-ready: rustia trails mlua-luau by 3.829x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,276 | 6 / 10 | wasmi | 38.06x | 4.528x | 0.725x | not public-ready: rustia trails mlua-luau by 4.528x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 15.22x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.649x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.118x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 2.814x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.823x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.673x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.537x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.273x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.134x | 0 / 5 | linux-x86_64 | 5 workloads |
