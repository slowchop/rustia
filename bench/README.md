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

### Reproducible ephemeral Linux host

The frozen dense-table follow-up for baseline
`dac4ec2295f77587c79f04e4ff0a09cb99912f96` and candidate
`ed65077c2e779ec6a641b8ca2f8c8258b2508a2d` is encoded in
[`cloud/digitalocean-dense-ed65077.json`](cloud/digitalocean-dense-ed65077.json)
and the protocol-identical dedicated-CPU fallback
[`cloud/linode-dense-ed65077.json`](cloud/linode-dense-ed65077.json).
It predeclares the source hashes, fixed work, repetitions, eight-leg order,
bootstrap interpretation, candidate-only stability qualification, Linux host
gate, CPU pin, cooldown, and conditional three-run publication rule.

DigitalOcean authentication is a one-time interactive operation. The token is
entered at `doctl`'s prompt and must never be passed as a recipe argument,
environment variable, repository file, or chat message:

```bash
just bench-do-auth
```

The recipe removes only the locally stored `rustia-bench` context before
initializing it, so an expired or replaced token always produces a fresh prompt.

When authenticating from another computer, run the prompt on the benchmark
coordinator host over a TTY-preserving SSH session:

```bash
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-do-auth'
```

The short-lived custom token needs only `droplet:create`, `droplet:read`,
`droplet:delete`, `regions:read`, `sizes:read`, `actions:read`, `image:read`,
`ssh_key:create`, `ssh_key:read`, and `ssh_key:delete`. The last three allow the
workflow to import and remove a unique per-run ED25519 key instead of depending
on a persistent account key.

The read-only plan resolves the mutable Ubuntu image slug to a numeric image ID
and requires exactly one currently available 2-vCPU/4-GiB/80-GiB
`Basic Intel` size in `syd1` at or below the committed hourly-price ceiling.
It fails rather than silently choosing a regular Basic, dedicated, larger,
differently priced, or alternate-region VM:

```bash
just bench-do-plan
```

Two tracked diagnostic profiles compare the useful shared-CPU alternatives:
regular Basic and Premium Intel, both with 2 vCPUs and 4 GiB of RAM. The
acceptance config freezes the qualifying cost-aware matrix winner; rerun the
read-only plans and billable trials by profile name when price, availability,
or host consistency needs to be reevaluated:

```bash
just bench-do-trial-plan basic-2c4g
just bench-do-trial-plan premium-intel-2c4g
just bench-do-trial basic-2c4g
just bench-do-trials
```

`bench-do-trials` intentionally runs the two profiles sequentially so their
cold setup/build cost can be compared; the two committed 90-minute ceilings
sum to $0.116070 at current rates. Each trial builds the
same exact baseline and candidate, runs only the three candidate qualification
legs, records CPU identity, build time, stability spread, billable lifetime,
conservative charge upper bound, and projected full-acceptance charge, then
confirms destruction. A failed qualification is reported as ineligible, and
the matrix fails if neither profile qualifies. Diagnostic mode is validated
independently on the guest and coordinator: it cannot run the
eight-leg comparison, append history, or promote evidence into the repository.
The 2-GiB, extra-disk, and 4-vCPU shared shapes are excluded because their
sub-cent savings or extra parallel build capacity do not repay the added OOM
risk or hourly rate during the protocol's fixed cooldown windows.
These trials screen one discarded shared host per profile; the eventual
acceptance command provisions a new host and repeats qualification rather than
treating a trial as a guarantee about future shared-CPU placement.

The all-in-one billable command creates one uniquely named Droplet, records its
numeric ID before proceeding, transfers a local Git bundle (so neither frozen
commit needs to be pushed), keeps equal-length source roots alive for the
runtime workload lookup, overlays only the candidate `bench/src/main.rs` onto
the baseline, builds both binaries once with the pinned toolchain, and fetches
the result archive before destroying the VM:

```bash
just bench-do
```

Before creating anything billable, it confirms a positive sentinel ID is absent
from both account lists, then sends no-op delete-authority probes for that ID.
A missing Droplet or SSH-key delete scope therefore fails before provisioning
instead of being discovered during teardown.

The acceptance and diagnostic profiles have a committed 90-minute
billable-lifecycle limit. The limit starts before Droplet creation, bounds every
remote phase, and reserves the final eight minutes for cleanup. The plan prints
the corresponding price-ceiling estimate. A provider or API outage can still
delay confirmed deletion, so cleanup failure remains a hard command error
rather than being reported as a stopped bill.

Qualification sidecars never enter the accepted A/B analysis. Every accepted
leg runs on CPU 1 after the committed idle window and cooldown; any declared
contaminating-process or steal violation discards the whole eight-leg block. Only
`material_regression_excluded` permits the same frozen candidate binary to
append three real rows. An accepted run adds the evidence directory and exact
append-only history locally, regenerates the history-derived reports, and runs
the benchmark artifact checks. An inconclusive, regressing, unstable, or
contaminated run leaves diagnostics under `.rustia-benchmark-state/` and does
not alter public history.

The coordinator confirms destruction through a successful API list, not from
a failed request or power-off state. Ambiguous create/import responses get a
bounded unique-name visibility window before absence can be recorded, and an
exclusive local coordinator lock rejects accidental simultaneous billable
runs. The ownership journal and recovery artifacts live under the ignored
`.rustia-benchmark-state/` tree, outside Cargo's disposable `target/` cache, so
`cargo clean` cannot erase them during a run. They support inspection and
exact-ID recovery after a client crash:

```bash
just bench-do-status
just bench-do-unlock                # only after an ungraceful coordinator crash
just bench-do-destroy               # when exactly one owned run remains
just bench-do-destroy RUN_ID        # choose an exact owned run
```

Repeated Premium Intel shared-CPU placements passed the idle gate but accrued
nonzero hypervisor steal during timed legs, including one placement that stayed
clean until the eighth comparison leg. Those blocks were correctly discarded.
DigitalOcean exposes no dedicated-CPU size to this account, so the acceptance
fallback uses Akamai/Linode `g6-dedicated-2`: 2 dedicated vCPUs, 4 GiB RAM, and
80 GiB SSD in Sydney. It keeps every source, workload, order, cooldown,
qualification, host-gate, bootstrap, verdict, and publication field frozen.

Create a short-lived personal access token with only **Linodes: Read/Write** and
**Account: Read Only**. Account read access lets the coordinator reject an
account-wide automatic-backup default before creating a billable instance. The
token is entered through a private TTY prompt and stored mode `0600` only under
the ignored `.rustia-benchmark-state/linode/` directory:

```bash
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-linode-auth'
ssh ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-linode-plan'
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-linode'
```

The Linode coordinator passes a unique local ED25519 public key directly in the
create request, so no persistent account SSH-key resource or additional token
scope is needed. Before billing it validates the exact public Ubuntu image,
dedicated type dimensions, regional availability and price, disabled automatic
backups, source hashes, clean tracked checkout, and delete authority. It
journals the unique label before creation, reconciles ambiguous responses only
by that label, and confirms the numeric instance ID is absent before allowing
local evidence promotion.

Linode rounds each instance lifetime up to whole hours. The committed 55-minute
lifecycle therefore has a one-hour `$0.054` compute ceiling at the selected
price and reserves its final eight minutes for cleanup; crossing the provider's
hour boundary or a delayed API deletion can add another billed hour. A separate
diagnostic run is intentionally skipped because it would incur another full
hour minimum and acceptance already begins with the same three candidate-only
qualification legs.

Crash recovery is provider-isolated:

```bash
just bench-linode-status
just bench-linode-unlock
just bench-linode-destroy
just bench-linode-destroy RUN_ID
```

The resulting evidence is Linux/x86_64-specific. It does not replace a
separate Apple-Silicon/macOS measurement of the earlier M4 observation.

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
