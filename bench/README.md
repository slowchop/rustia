# script-bench-harness

`script-bench-harness` is a standalone benchmark harness for embedded scripting
engines. It lives in the rustia repo while rustia is early, but it is structured
to split into its own OSS project with a website, published result history, and
stats charts.

Split notes are tracked in [`STANDALONE.md`](STANDALONE.md).

Start with [`SCOREBOARD.md`](SCOREBOARD.md) for the latest human-readable
target-by-target comparison. It includes native and wasm targets, aggregate
score, ratio to Rustia, measured workload counts, and unsupported adapter rows.
Use [`PERF_PROGRESS.md`](PERF_PROGRESS.md) for the working log of kept wins,
reverted VM experiments, and next performance bets.

The harness writes one JSONL record per completed engine/workload measurement,
so history can be committed, served statically, or uploaded by CI without needing
a hosted metrics service.

The default build runs the in-repo `rustia` adapter over five workload families:

- `dense_tables`
- `sparse_tables`
- `closures_upvalues`
- `generic_iteration`
- `stdlib_math_table`

The inventory currently contains 12 workload entries. Non-default workloads are
kept out of the committed current-results snapshot until they have intentional
measurement history; `string_building` is wired across every requested adapter
as the #15 string/concat workload source set.

```sh
cargo run -p script-bench-harness --release -- --history bench/history/runs.jsonl
```

List all configured engine slots:

```sh
cargo run -p script-bench-harness -- --list
```

Emit the same engine/workload inventory as deterministic JSON for wrappers,
CI, or static-site generation:

```sh
cargo run -p script-bench-harness -- --list-json
```

Emit the wasm pre-lock gate tracker as deterministic JSON:

```sh
cargo run -p script-bench-harness -- --wasm-gate-json
```

Emit static Rustia bytecode counts for selected Luau workloads. This is useful
for spotting table, atom, inline-cache, and quickening candidates before adding
new VM hot-path machinery:

```sh
cargo run -p script-bench-harness -- --rustia-bytecode-summary
```

Measure the `call_ref` versus `br_table` wasm dispatch gate across available
JavaScript runtimes:

```sh
node tools/run-wasm-dispatch-gate.js
```

Engine slots are tracked for `rustia`, `boa`, `koto`, `mlua-lua54`,
`mlua-luau`, `rhai`, `roto`, `rquickjs`, `wasmi`, and `wasmtime`. Adapters are
feature-gated so heavyweight dependencies do not affect the normal workspace
build.

The matrix switch is present now:

```sh
cargo run -p script-bench-harness --features all-engines -- --list
```

Run and append the full default benchmark matrix with:

```sh
just benchmarks
```

That recipe runs the `all-engines` binary first, runs the Lua 5.4 mlua adapter
in its required separate binary, appends measurements to `bench/history/runs.jsonl`,
and refreshes the generated result files. It intentionally requires a clean
staged/unstaged worktree before appending benchmark history, so committed
baselines do not get tagged with `-dirty` commit IDs. The recipe pins the clean
HEAD commit into both benchmark invocations, because the first append dirties
`bench/history/runs.jsonl` before the separate Lua 5.4 binary runs.

GitHub Actions runs the same measurement machinery in
`.github/workflows/benchmarks.yml`. Pushes to `main` run a Rustia-only benchmark
for commit-level signal. A nightly schedule runs the full comparison matrix, and
manual dispatch can choose either `rustia` or `full` mode. The workflow requires
a self-hosted runner with the generic labels `[self-hosted, bench]`, regenerates
the benchmark artifacts, uploads them as workflow artifacts, and publishes
trusted runs to `slowchop/rustia-bench-data` with `BENCH_DATA_TOKEN`. Pushes to
`main` and scheduled full runs publish automatically; manual dispatch can opt in
with `publish=true`. The runner-label, trigger, and mode policy is checked by
`tools/check-workflows.js`.

`all-engines` intentionally excludes `mlua-lua54` because `mlua-lua54` and
`mlua-luau` cannot be linked into one binary. Run the Lua 5.4 mlua adapter as a
separate binary:

```sh
cargo run -p script-bench-harness --no-default-features --features mlua-lua54 -- --engines mlua-lua54
```

In this slice, `rustia` works through the default `rustia-engine` feature,
`boa` works with `--features boa`, `koto` works with `--features koto`, Lua 5.4
works with `--features mlua-lua54`, the Luau oracle comparison works with
`--features mlua-luau`, Rhai works with `--features rhai-engine`, Roto works
with `--features roto`, QuickJS works with `--features rquickjs-engine`, Wasmi
works with `--features wasmi-engine`, and Wasmtime works with
`--features wasmtime-engine`. The crate also builds with `--no-default-features`
so the standalone harness shell is not tied to rustia internals.

## History Schema

Each line in `history/runs.jsonl` is a JSON object:

```json
{
  "timestamp_unix": 0,
  "commit": "8c0211d",
  "target": "macos-aarch64",
  "engine": "rustia",
  "dialect": "luau",
  "workload": "dense_tables",
  "unit": "build+sum+sort of 256-elem array",
  "n": 1000,
  "reps": 9,
  "ns_per_unit_min": 1.0,
  "ns_per_unit_median": 1.1,
  "units_per_sec": 1000000.0,
  "checksum": "number:123"
}
```

The future website should treat JSONL as the append-only source of truth and
generate charts from `engine × target × workload` series.

`--list-json` emits a separate schema-identified metadata document for the
available engine slots, feature gates, workload IDs, default workload set, and
result paths. It is intended for tooling; benchmark measurements stay in the
append-only JSONL history above. In that document, `implemented` means the
source adapter exists, while `available` means this specific binary was compiled
with the adapter's feature enabled.

Validate committed history without running benchmarks:

```sh
cargo run -p script-bench-harness -- --check-history
```

The check validates required fields, known engine/workload IDs, dialect and unit
consistency, positive finite timing values, clean commit IDs, and append-only
timestamp order.

Refresh generated markdown results from committed history without running
benchmarks:

```sh
cargo run -p script-bench-harness -- --refresh-results
```

By default, the same command also writes `bench/current-results.json`, a
deterministic machine-readable current-results snapshot keyed by target,
engine, and workload.
Use `--latest-json <path>` to choose a different destination or
`--no-latest-json` to skip it.

Generate the static current-results HTML from the latest snapshot:

```sh
node bench/render-current-results.js
```

Or refresh the committed current-results JSON and render the HTML together:

```sh
just gen-current-results
```

`gen-current-results` does not run measurements; it only regenerates result
artifacts from committed history, including [`CLAIMS.md`](CLAIMS.md), the
per-workload public-claim gate. `gen-benchmark-results` is kept as a recipe
alias.

Check that committed benchmark markdown, current-results JSON, claim gate, and
current HTML are fresh without running measurements:

```sh
just check-benchmark-results
```

Pass `--commit <id>` only for orchestrated measurement runs that already proved
the tree was clean before the first history append.

## Provenance

The harness is a clean rewrite. Prior-art and workload-provenance notes are
tracked in [`PRIOR-ART.md`](PRIOR-ART.md).

## Static Site

`site/` contains a dependency-free dashboard that reads `history/runs.jsonl`,
shows current leaderboards, renders per-workload history charts, and loads
`wasm-gate.json` so the closed V8/JSC/SpiderMonkey pre-lock requirements stay
visible next to the rough performance comparisons.
It also reads `wasm/current-results.json` for the browser-wasm target bench.
The dispatch panel reads `wasm/dispatch-current-results.json` for the
`call_ref` versus `br_table` microbench.
That target currently measures the wasm-compatible scripting adapters `rustia`
and `koto`; current adapter status is tracked in
[`wasm/ADAPTERS.md`](wasm/ADAPTERS.md).

```sh
python3 -m http.server 8000
```

Open `http://127.0.0.1:8000/bench/site/`.
