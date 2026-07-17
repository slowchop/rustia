# script-bench-harness

`script-bench-harness` is a standalone benchmark harness for embedded scripting
engines. It lives in the rustia repo while rustia is early, but it is structured
to split into its own OSS project with a website, published result history, and
stats charts.

Split notes are tracked in [`STANDALONE.md`](STANDALONE.md).

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

Each calibration probe and each recorded repetition uses a freshly prepared
engine/workload instance. A single invocation at the selected `n` warms that
instance before the timer starts, then exactly one invocation is timed. This
keeps growing VM state and caches from leaking across repetitions while leaving
engine setup, source compilation, and warmup outside the sample.

Pass `--fixed-n <positive integer>` to bypass calibration and use exactly that
`n` for every warmup and recorded repetition. Fixed mode also bypasses
`--max-n`; `--target-ms` is irrelevant because no calibration probe runs. The
default remains calibrated measurement, so existing commands and public history
behavior are unchanged.

The append-only history continues to store only the stable min/median summary.
For local analysis, write the raw per-repetition `ns/unit` values to a separate
JSON sidecar:

```sh
cargo run -p script-bench-harness --release -- \
  --engines rustia \
  --samples-json /tmp/rustia-samples.json \
  --no-history --no-results --no-latest-json
```

The sidecar uses the `rustia.bench.raw-samples.v1` schema and preserves samples
in execution order. It also records the commit, target, workload, selected `n`,
whether `n` was `calibrated` or `fixed`, the checksum, and summary statistics
for every measurement. Keep separate baseline and candidate sidecars and
alternate their run order (for example, ABBA then BAAB) when evaluating small
changes, so machine drift is visible rather than folded into one best result.
Fixed-N runs that append history must provide `--samples-json`; the harness
writes that sidecar before appending its summary row.

The 2026-07-17 local table/cache/`GETIMPORT`/`FASTCALL` hardening comparison is
recorded in [`../design/56-performance-research.md`](../design/56-performance-research.md),
including the baseline and harness hashes, broad ABBA medians, cooled focused
checks, and the deliberately conservative claim posture.

## Equal-work validation and publication

Use fixed mode when a small A/B difference must not be confounded by separate
calibration choices. Build the baseline and candidate from byte-identical
`bench/src/main.rs` source, then run the following from a quiet benchmark host.
The example fixes the dense-table workload at 1,500 units and 15 repetitions;
choose and record a workload-appropriate fixed value for any other workload.
Freeze a clean candidate commit before creating any accepted evidence. Build
both release binaries only after freezing their inputs. Do not rebuild or
replace the candidate after accepting the A/B comparison: its exact SHA-256
binary is also the one used for final publication. Also predeclare a numeric
host-idle/thermal policy and an external-contamination rule before the first
leg. If either rule is violated, discard and repeat the whole eight-leg block;
never include or exclude a leg based on its timing result.

```bash
git diff --quiet --ignore-submodules --
git diff --cached --quiet --ignore-submodules --
test -z "$(git ls-files --others --exclude-standard -- bench crates Cargo.toml Cargo.lock rust-toolchain.toml)"

baseline_bin=/absolute/path/to/baseline/target/release/script-bench
candidate_bin=/absolute/path/to/candidate/target/release/script-bench
baseline_commit=BASELINE_COMMIT
candidate_commit="$(git rev-parse HEAD)"
evidence_dir="bench/history/evidence/$candidate_commit"
bootstrap_reps=20000
bootstrap_seed=1381323604
material_threshold=5
common=(--engines rustia --workloads dense_tables --fixed-n 1500 --reps 15 --no-history --no-results --no-latest-json)

"$baseline_bin" "${common[@]}" --commit "$baseline_commit" --samples-json "$evidence_dir/dense-abba-A1.json"
"$candidate_bin" "${common[@]}" --commit "$candidate_commit" --samples-json "$evidence_dir/dense-abba-B1.json"
"$candidate_bin" "${common[@]}" --commit "$candidate_commit" --samples-json "$evidence_dir/dense-abba-B2.json"
"$baseline_bin" "${common[@]}" --commit "$baseline_commit" --samples-json "$evidence_dir/dense-abba-A2.json"
```

Let the host return to the same idle and thermal state between legs. Repeat in
the reverse order rather than treating the first sequence as conclusive:

```bash
"$candidate_bin" "${common[@]}" --commit "$candidate_commit" --samples-json "$evidence_dir/dense-baab-B3.json"
"$baseline_bin" "${common[@]}" --commit "$baseline_commit" --samples-json "$evidence_dir/dense-baab-A3.json"
"$baseline_bin" "${common[@]}" --commit "$baseline_commit" --samples-json "$evidence_dir/dense-baab-A4.json"
"$candidate_bin" "${common[@]}" --commit "$candidate_commit" --samples-json "$evidence_dir/dense-baab-B4.json"
```

Analyze all eight legs with the checked, dependency-free analyzer. Its seeded
bootstrap, seed, and material-regression threshold are explicit so another
machine can reproduce the interpretation exactly:

```bash
node bench/analyze-fixed-samples.js \
  --baseline "$evidence_dir/dense-abba-A1.json" \
  --baseline "$evidence_dir/dense-abba-A2.json" \
  --baseline "$evidence_dir/dense-baab-A3.json" \
  --baseline "$evidence_dir/dense-baab-A4.json" \
  --candidate "$evidence_dir/dense-abba-B1.json" \
  --candidate "$evidence_dir/dense-abba-B2.json" \
  --candidate "$evidence_dir/dense-baab-B3.json" \
  --candidate "$evidence_dir/dense-baab-B4.json" \
  --bootstrap-reps "$bootstrap_reps" \
  --seed "$bootstrap_seed" \
  --material-threshold "$material_threshold" \
  --expected-legs 4 \
  --require-full-commits \
  > "$evidence_dir/analysis.json"
```

The analyzer validates the exact sidecar protocol metadata and summaries, four
unique legs per group, distinct lowercase full commit IDs, fixed-work identity,
repetitions, and checksum. It reports per-leg medians, pooled five-number
summaries, and nominal 95% pooled and leg-clustered hierarchical bootstrap
intervals. The verdict uses their conservative envelope; neither interval can
cover unobserved host states, which is why the predeclared host gate remains
mandatory. Proceed only when the verdict is `material_regression_excluded`; a
confirmed regression or an inconclusive interval requires investigation or
more predeclared evidence.

These sidecars are diagnostic evidence, not public-history records. Do not
manufacture JSONL rows from pooled samples or copy a computed median into
`history/runs.jsonl`. Once a final commit is accepted, produce repeatable public
history from the actual clean final-commit binary. For this protocol,
"repeatable" means at least three independent final-commit measurements on the
same controlled target. Recheck that the frozen commit and tracked inputs are
unchanged before the first append; the already-created evidence directory is
the only allowed untracked benchmark input. Run each command only after the
host has cooled:

```bash
test "$(git rev-parse HEAD)" = "$candidate_commit"
git diff --quiet --ignore-submodules --
git diff --cached --quiet --ignore-submodules --
test -z "$(git ls-files --others --exclude-standard -- bench crates Cargo.toml Cargo.lock rust-toolchain.toml ':(exclude)bench/history/evidence/**')"
final_commit="$candidate_commit"
evidence_dir="bench/history/evidence/$final_commit"
test -x "$candidate_bin"
publish=(--engines rustia --workloads dense_tables --fixed-n 1500 --reps 15 --history bench/history/runs.jsonl --no-results --no-latest-json --commit "$final_commit")

"$candidate_bin" "${publish[@]}" --samples-json "$evidence_dir/final-dense-1.json"
"$candidate_bin" "${publish[@]}" --samples-json "$evidence_dir/final-dense-2.json"
"$candidate_bin" "${publish[@]}" --samples-json "$evidence_dir/final-dense-3.json"

env -u RUSTC_WRAPPER cargo +1.95.0 run -p script-bench-harness -- --check-history
env -u RUSTC_WRAPPER cargo +1.95.0 run -p script-bench-harness -- --refresh-results
node bench/render-current-results.js
node bench/render-claims.js
just check-benchmark-results
```

The explicit `--commit` is valid here only because the tree was checked before
the first append; the history file itself makes the worktree dirty afterward.
Review the three raw sidecars before committing generated history and reports.
If their leg medians, spread, or checksums disagree unexpectedly, keep the claim
unpublished and investigate instead of selecting the most favorable run.

Record the following provenance beside any published interpretation:

- full engine commits for baseline and candidate, plus SHA-256 values for the
  byte-identical harness source, selected workload source, and release binaries;
- pinned Rust version, target, OS/CPU, runner identity, and power/thermal policy;
- complete command, workload, fixed `n`, repetitions, run order, timestamps,
  and cooldown policy;
- raw-sidecar paths and SHA-256 values, sample counts, and workload checksums;
- every leg median and spread, the comparison method and confidence interval,
  and the material-regression threshold chosen before inspecting the result.

The append-only JSONL remains the summary source of truth. Raw sidecars and a
provenance manifest live under [`history/evidence/`](history/evidence/) and must
not change that public schema.

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
a self-hosted runner with the generic labels `[self-hosted, bench]`. It restores
prior history from the private `slowchop/rustia-bench-data` repository, appends
new measurements, regenerates the benchmark reports, and commits the updated
result files back to that data repository. It also regenerates the data repo's
root `README.md` with latest-results comparison charts split by native and wasm
targets, so adapters such as `koto` are not conflated across target families.
The runner-label, trigger, storage, and mode policy is checked by
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

Raw repetitions are intentionally excluded from this public history schema.
`--samples-json <path>` exports them without changing JSONL history records or
generated current-results artifacts. A sidecar is optional for ordinary local
or calibrated runs, but required when a fixed-N run appends history; accepted
milestone evidence is retained under `history/evidence/<full-commit>/`.

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

Generate the static benchmark HTML report from the latest snapshot and committed
history:

```sh
node bench/render-current-results.js
```

Or refresh the committed current-results JSON and render the HTML together:

```sh
just gen-current-results
```

`gen-current-results` does not run measurements; it only regenerates result
artifacts from committed history, including [`current-results.html`](current-results.html)
with the current leaderboard and history charts, plus [`CLAIMS.md`](CLAIMS.md),
the per-workload public-claim gate. `gen-benchmark-results` is kept as a recipe
alias.

Run the fixed-sample analyzer self-test and check that committed benchmark
markdown, current-results JSON, claim gate, and current HTML are fresh without
running measurements:

```sh
just check-benchmark-results
```

Pass `--commit <id>` only for orchestrated measurement runs that already proved
the tree was clean before the first history append.

## Provenance

The harness is a clean rewrite. Prior-art and workload-provenance notes are
tracked in [`PRIOR-ART.md`](PRIOR-ART.md).

## Static Site

`current-results.html` is the single generated report for current leaderboards,
per-workload history charts, wasm results, and claim-gate context.

`site/` contains the older dependency-free dashboard that reads `history/runs.jsonl`
directly. It remains useful while editing the raw JSONL history by hand.
It also loads `wasm-gate.json` so the closed V8/JSC/SpiderMonkey pre-lock
requirements stay visible next to the rough performance comparisons.
The dashboard also reads `wasm/current-results.json` for the browser-wasm target bench.
The dispatch panel reads `wasm/dispatch-current-results.json` for the
`call_ref` versus `br_table` microbench.
That target currently measures the wasm-compatible scripting adapters `rustia`
and `koto`; current adapter status is tracked in
[`wasm/ADAPTERS.md`](wasm/ADAPTERS.md).

```sh
python3 -m http.server 8000
```

Open `http://127.0.0.1:8000/bench/site/`.
