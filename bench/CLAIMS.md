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
- Measurement window: 2026-07-22 04:08:05 UTC to 2026-07-22 04:12:38 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 590 | 9 / 12 | roto | 39.05x | 6.667x | 1.037x | not public-ready: rustia trails mlua-luau by 6.667x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 175,068 | 8 / 12 | wasmtime | 273x | 4.995x | 0.422x | not public-ready: rustia trails mlua-luau by 4.995x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 22,076 | 10 / 12 | wasmtime | 168x | 3.265x | 1.892x | not public-ready: rustia trails mlua-luau by 3.265x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 159,265 | 7 / 12 | wasmtime | 199x | 4.246x | 0.647x | not public-ready: rustia trails mlua-luau by 4.246x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 2,271 | 9 / 12 | wasmtime | 223x | 6.306x | 0.96x | not public-ready: rustia trails mlua-luau by 6.306x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 120x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 23.38x | 0 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.938x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 4.93x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.871x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 1.646x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.293x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 1.027x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.875x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.436x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.199x | 0 / 5 | linux-x86_64 | 5 workloads |
