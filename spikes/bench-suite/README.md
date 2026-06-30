# rustia broad benchmark suite + reference-Luau baseline ([H2])

Measures **THE BAR** rustia must beat. rustia does not exist yet — these numbers are the
reference-Luau target (and the koto incumbent it replaces), NOT rustia.

## Oracle versions
- Luau: `luau0-src 0.18.3+luau709` (Luau **0.709**) via `mlua 0.11.6` (`luau` + `vendored`), `_VERSION="Luau 0.709"`.
- koto: `0.16.1`, `default-features=false, features=["arc"]` (matches grabby's `ai_bakeoff`/`scenario`).
- Host: aarch64-mac. Builds: `--release`, opt-level=3, LTO=fat, codegen-units=1, single thread.

## Run
```
cargo run --release --manifest-path Cargo.toml            # Luau baseline (11 workloads)
cargo run --release --manifest-path koto-harness/Cargo.toml   # koto baseline (subset)
cargo run --release --manifest-path koto-harness/Cargo.toml -- probe   # koto API probe
```

## Method
Each workload is a `.luau`/`.koto` file that `return`s/`export`s `f(n)` doing **n internal "units"**
of representative work and returning a checksum (prevents dead-code elision; keeps results live).
The harness makes ONE host->VM call per measurement, so FFI/marshalling overhead is amortised to ~0 and
the number is pure in-VM throughput. n is auto-calibrated to ~0.25 s/rep; we report **min of N reps**
(min = least scheduler noise). Metric = **ns per unit**; the "unit" differs per workload, so compare a
future rustia run against this table **per-workload**, never across workloads.

## Workloads (and koto feasibility)
| # | workload | unit | koto? |
|---|----------|------|-------|
|01|dense_tables|build+sum+sort 256-elem array|yes (list+sort)|
|02|sparse_tables|fill+read 128 hash entries|yes (map)|
|03|oop_metamethods|`(p+v)*s` + `:dot` + `:len2`|feasible via `@meta` maps; not ported (effort)|
|04|closures_upvalues|one upvalue-mutating closure call|yes (semantics differ, see notes)|
|05|string_patterns|gmatch+match+gsub over a line|**NO koto analog** (no Lua pattern engine)|
|06|string_format|one mixed-spec `string.format`|feasible via `"{}".format`; not ported (brace/semantics)|
|07|coroutines|one resume->yield round trip|yes (koto generators)|
|08|pcall_xpcall|pcall+xpcall pair (1/16 throws)|partial: koto `try/catch`, ONE mechanism (no xpcall)|
|09|generic_iteration|`__iter`(32)+pairs(5)+ipairs(32)|yes (for-in over list/map)|
|10|stdlib_math_table|math fastcalls + buffered push/sort|yes (number methods)|
|11|serialization|serialize+parse 32-record round trip|feasible (manual); not ported (no patterns => verbose)|

Caveats: dense/sparse include per-unit table allocation + GC churn (representative of real code).
koto numbers use native i64/f64 (the determinism-UNSAFE float path, mirroring `ai_bakeoff` SCRIPT_FLOAT);
the grabby DETERMINISTIC koto path adds host `fxmul`/`curve` calls and is materially slower (bakeoff: ~30-40x native).
pcall unit is 2 protected calls in Luau vs 1 in koto — divide koto's column by ~2 for a like-for-like.
