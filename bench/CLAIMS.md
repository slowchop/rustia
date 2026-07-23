# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 60
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-22 11:41:46 UTC to 2026-07-22 11:46:20 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 229 | 5 / 10 | mlua-lua54 | 4.951x | 3.553x | 0.685x | not public-ready: rustia trails mlua-luau by 3.553x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 54,467 | 5 / 10 | wasmi | 17.99x | 2.805x | 0.19x | not public-ready: rustia trails mlua-luau by 2.805x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 12,423 | 8 / 10 | wasmi | 37.13x | 2.796x | 1.801x | not public-ready: rustia trails mlua-luau by 2.796x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 90,337 | 5 / 10 | wasmi | 28.05x | 3.83x | 0.618x | not public-ready: rustia trails mlua-luau by 3.83x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,175 | 7 / 10 | wasmi | 41.97x | 4.88x | 0.682x | not public-ready: rustia trails mlua-luau by 4.88x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 15.81x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.833x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.494x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.195x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.972x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.767x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.629x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.323x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.156x | 0 / 5 | linux-x86_64 | 5 workloads |
