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
- Measurement window: 2026-07-22 05:11:23 UTC to 2026-07-22 05:15:55 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 250 | 7 / 12 | roto | 42.53x | 3.95x | 0.76x | not public-ready: rustia trails mlua-luau by 3.95x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 55,686 | 6 / 12 | wasmtime | 103x | 2.753x | 0.194x | not public-ready: rustia trails mlua-luau by 2.753x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 10,704 | 9 / 12 | wasmtime | 88.75x | 2.331x | 1.576x | not public-ready: rustia trails mlua-luau by 2.331x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 85,408 | 7 / 12 | wasmtime | 160x | 3.746x | 0.594x | not public-ready: rustia trails mlua-luau by 3.746x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,121 | 8 / 12 | wasmtime | 162x | 4.374x | 0.678x | not public-ready: rustia trails mlua-luau by 4.374x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 71.81x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 14.38x | 0 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.729x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.339x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.183x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 2.075x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.937x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.755x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.622x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.301x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.156x | 0 / 5 | linux-x86_64 | 5 workloads |
