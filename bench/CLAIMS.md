# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 50
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-21 23:30:01 UTC to 2026-07-21 23:33:34 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 644 | 8 / 10 | roto | 110x | 10.02x | 1.924x | not public-ready: rustia trails mlua-luau by 10.02x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 164,193 | 6 / 10 | wasmtime | 304x | 8.54x | 0.569x | not public-ready: rustia trails mlua-luau by 8.54x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 31,037 | 9 / 10 | wasmtime | 257x | 7.101x | 4.572x | not public-ready: rustia trails mlua-luau by 7.101x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 114,874 | 7 / 10 | wasmtime | 216x | 5.087x | 0.749x | not public-ready: rustia trails mlua-luau by 5.087x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,579 | 7 / 10 | wasmtime | 228x | 6.648x | 0.884x | not public-ready: rustia trails mlua-luau by 6.648x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 151x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 33.44x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 7.288x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 6.641x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 4.456x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.978x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 1.271x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.649x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.327x | 0 / 5 | linux-x86_64 | 5 workloads |
