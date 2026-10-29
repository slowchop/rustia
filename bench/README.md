# script-bench-harness

## Measurement policy and paused source profile

This source checkout uses hosted correctness CI and has no benchmark workflow.
Public per-commit/history measurement, landscape refresh, and publication remain
paused. A separate private maintainer bridge supports manually dispatched,
reviewed exact-cohort runtime comparisons only. Outputs stay private, require
independent raw-data review, and do not update public history or authorize public
performance claims. See [the bridge contract](https://github.com/slowchop/rustia/blob/main/design/81-benchmark-operations.md#public-source-profile).
Displayed results are retained historical evidence, not a current host-health
or repeatability guarantee. See [CURRENT](https://github.com/slowchop/rustia/blob/main/design/CURRENT.md) and the
[source profile](https://github.com/slowchop/rustia/blob/main/design/public-source-profile.json).

**Do not run timings or profiling on a development machine.** This includes
benchmark binaries, examples, Wasm timing scripts, and ad hoc comparisons.
Only accepted `ci-bench` results count as performance evidence. Bridge availability
alone does not establish a result; leave unperformed measurements pending.

The measurement commands and workflow names below document retained protocols
and the historical private coordinator. Timing commands are reference material
for trusted CI operators, not local setup instructions or workflows present in
this source profile. Local compilation, offline artifact validation, and rendering
already-recorded data are allowed; `just check-benchmark-results` validates artifacts
without measuring performance.

## Harness and retained protocols

`script-bench-harness` is a standalone benchmark harness for embedded scripting
engines. It lives in the rustia repo while rustia is early, but it is structured
to split into its own OSS project with a website, published result history, and
stats charts.

Split notes are tracked in [`STANDALONE.md`](STANDALONE.md).

The harness writes one JSONL record per completed engine/workload measurement,
so history can be committed, served statically, or uploaded by CI without needing
a hosted metrics service.

The retained owned-host measurement protocol is narrower than Rustia's build
support: native measurements are `linux-x86_64` on the owned benchmark host
labelled `ci-bench`, and the wasm runtimes are measured under Node/V8 on that
same host, inside the cross-engine matrix lane. macOS and Windows timings are
excluded until dedicated machines exist for them.

The default build runs the in-repo `rustia` adapter over five workload families:

- `dense_tables`
- `sparse_tables`
- `closures_upvalues`
- `generic_iteration`
- `stdlib_math_table`

The documented inventory contained 20 workload entries. Non-default workloads are
kept out of the committed timing snapshot until they have intentional
measurement history; `string_building` is wired across every requested adapter
as the #15 string/concat workload source set. Entries #16–#18 are portable
burst/churn/intentional-retention memory probes, and #19–#21 are deterministic
steady/history/intentional-retention todo-app memory workloads for Luau and
Koto.

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

The JSONL summary stores only the stable min/median values. Historical trusted
CI runs could append rows, while the private matrix coordinator
(`weekly-matrix-bench.yml`, through `tools/run-dedicated-benchmark.js matrix`)
published a complete Linux cohort atomically. That workflow is absent from this
source profile. Harness executions used `--samples-json <path>` to retain raw
per-repetition `ns/unit` values in a separate sidecar for offline analysis.

The sidecar uses the `rustia.bench.raw-samples.v1` schema and preserves samples
in execution order. It also records the commit, target, workload, selected `n`,
whether `n` was `calibrated` or `fixed`, the checksum, and summary statistics
for every measurement. Keep separate baseline and candidate sidecars and
alternate their run order (for example, ABBA then BAAB) when evaluating small
changes, so machine drift is visible rather than folded into one best result.
Fixed-N runs that append history must provide `--samples-json`; the harness
writes that sidecar before appending its summary row.

The dated hardening diagnostics, 2026-07-20 accepted dense/OOP comparisons, and completed string-memory evidence are
recorded in [`design/56-performance-research.md`](https://github.com/slowchop/rustia/blob/main/design/56-performance-research.md),
including exact identities, paired protocols, confidence intervals, retained artifacts, and the deliberately
conservative claim posture.

## Memory benchmark direction

The existing `vm-string-memory` sampler is a focused Rustia A/B evidence tool, not a general engine comparison.
The first-class memory product is specified in
[`design/57-memory-benchmarking.md`](https://github.com/slowchop/rustia/blob/main/design/57-memory-benchmarking.md). Its child/supervisor, cgroup-v2
sampling, repeated-epoch analyzer, synthetic controls, deterministic todo workloads, separate history, and
pending/ready dashboard integration are now implemented; the published cohort is `rustia-ci-070c387b173c`
(11 rustia rows from 99 raw samples), measured on the owned host `ci-bench` on 2026-09-05. The earlier
cloud-measured cohorts, `pre-reclamation-18b58d8` (110 rows, 990 fresh processes) among them, were removed on
2026-09-06. Throughput and instrumented memory measurements remain separate.

```sh
just bench-memory-self-test
```

The historical private `.github/workflows/rustia-memory.yml` produced memory rows; it is absent
from this source profile. That dispatch-only workflow used a required `commit_sha`
input and a `cohort` choice — `rustia-only`, the default 11-row per-commit cohort, or `cross-engine`, the
110-row thirteen-engine cohort under the same protocol — on the `bench-solo` runner group, pinning two isolated
cores so the supervisor never shares a core with the benchmark. Its rows are pushed to the published data
repository and rendered by the shared publisher; there, each engine's rows come from the newest cohort that
measured it. Competitor rows and the mlua/Luau/JIT reference return with the first cross-engine cohort measured
on the owned host. The measured commit's own tools measure, but the merge into the data repository and the push
run main's copies, taken before the checkout moves, and a `commit_sha` from before a row's host was confined to a
label is refused before the build, for the reasons in
[`design/81-benchmark-operations.md`](https://github.com/slowchop/rustia/blob/main/design/81-benchmark-operations.md).

## Equal-work validation and publication

The retained acceptance protocol uses fixed mode so a small A/B difference is
not confounded by separate calibration choices. The following commands document
trusted CI execution on a qualified benchmark host, not a local procedure.
The baseline and candidate use byte-identical `bench/src/main.rs` source.
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

The historical accepted DigitalOcean VM Core 2 C2.2 comparison is frozen in
[`cloud/digitalocean-core2-84e732a-ams3.json`](cloud/digitalocean-core2-84e732a-ams3.json).
It compares exact baseline `9695a62` with candidate `84e732a` on
`generic_iteration` using 101-repetition sample blocks in the fixed-work paired
protocol and bounded idle/steal gates described below. Its exact AMS3 Basic
Intel 4-vCPU/8-GiB shape is frozen in the config; the plan fails instead of
substituting a different size or region. The accepted eight-leg comparison
measured 8.1651% lower candidate ns/unit with a conservative nominal 95%
interval of -10.2126% to -5.9577%. Its evidence was measured on a rented
Droplet and was removed on 2026-09-13; the samples remain in Git history.

The earlier Region Engine v1 comparison remains frozen in
[`cloud/digitalocean-region-b861ee9.json`](cloud/digitalocean-region-b861ee9.json).
The earlier dense-table follow-up for baseline
`dac4ec2295f77587c79f04e4ff0a09cb99912f96` and candidate
`ed65077c2e779ec6a641b8ca2f8c8258b2508a2d` is encoded in
[`cloud/digitalocean-dense-ed65077.json`](cloud/digitalocean-dense-ed65077.json)
and the accepted dedicated-CPU protocol
[`cloud/linode-dense-ed65077.json`](cloud/linode-dense-ed65077.json), plus the
actual-bare-metal fallback
[`cloud/vultr-dense-ed65077.json`](cloud/vultr-dense-ed65077.json), and the
exact-machine Scaleway fallback
[`cloud/scaleway-dense-ed65077.json`](cloud/scaleway-dense-ed65077.json).
It predeclares the source hashes, fixed work, repetitions, eight-leg order,
bootstrap interpretation, paired relative-stability qualification, Linux host
gate, CPU pin, cooldown, and conditional three-run publication rule.

The accepted OOP comparison is frozen separately in
[`cloud/linode-oop-f208c12.json`](cloud/linode-oop-f208c12.json). The evidence-only 512-VM string sampler is
frozen in
[`cloud/linode-vm-string-memory-1370d6c.json`](cloud/linode-vm-string-memory-1370d6c.json); it uses paired
fresh-process rounds and never appends throughput history.

The DigitalOcean, Vultr and Scaleway coordinators that ran these comparisons
have been deleted, the memory cohort has moved to the owned host, and the eleven
Linode lifecycle recipes are gone. Nothing in this repository authenticates to,
creates, or pays for a cloud instance -- not on a timer, not on a push, not by
dispatch, not by recipe. The operating instructions for those providers, which
named a developer machine by host alias, are gone with them.

The frozen configs behind published measurements stay. They are the protocol a
reader needs to check a number that is still on the dashboard; the tool that
once executed them being gone does not make the record less true. Configs no
published evidence referenced were deleted with their coordinator.

Qualification sidecars never enter the accepted A/B analysis. DigitalOcean v6
admits four counterbalanced baseline/candidate pairs by comparing the repeated
relative ratios within each matching order. This preserves the intended AB/BA
counterbalance instead of misclassifying a repeatable second-leg frequency
effect as host instability. Each pair gets one committed idle window and
cooldown; its second leg follows immediately, and both timed legs retain the
contaminant and steal checks. Idle admission requires zero steal; a timed leg
permits at most 0.5% steal. This shared-CPU ceiling remains one tenth of the
predeclared 5% material-regression threshold, while the within-order 5%
relative-stability gate rejects frequency shifts that exceed the claim the
accepted comparison must independently exclude. The eight accepted legs use
the same four-pair treatment. Any declared
contaminating-process or out-of-budget steal violation discards the whole block. Only
`material_regression_excluded` permits the same frozen candidate binary to
produce three real candidate rows, retained as raw sidecars in the evidence
directory. The local promotion leaves the complete single-revision public
matrix history unchanged, regenerates the dashboard with the accepted A/B
comparison, and runs the benchmark artifact checks. An inconclusive,
regressing, unstable, or
contaminated run leaves diagnostics under `.rustia-benchmark-state/` and does
not alter public history.

Two facts from that lane are worth keeping now that its tooling is gone. A
provider outage can outlast a coordinator's confirmation window and leave an
instance billing with nothing running to stop it: this happened on 2026-08-24,
when the API returned 503/504 for roughly twenty minutes and one instance
billed for about 1.4 hours. And repeated shared-CPU placements passed the idle
gate while still accruing hypervisor steal during timed legs, which is why the
owned host requires steal to be exactly zero rather than merely bounded --
on bare metal a non-zero reading is a fault, not a tolerance.

The Linode coordinator passes a unique local ED25519 public key directly in the
create request, so no persistent account SSH-key resource or additional token
scope is needed. Before billing it validates the exact public Ubuntu image,
dedicated type dimensions, public capacity, account-specific region access and
price, disabled automatic backups, source hashes, clean tracked checkout, and
delete authority. It journals the unique label before creation, reconciles
ambiguous responses only by that label, and confirms the numeric instance ID is
absent before allowing local evidence promotion.

Linode rounds each instance lifetime up to whole hours. Each committed 55-minute
lifecycle therefore has a one-hour `$0.054` compute ceiling at the selected
price and reserves its final eight minutes for cleanup; crossing the provider's
hour boundary or a delayed API deletion can add another billed hour. For the fixed-throughput configs, a separate
diagnostic run is intentionally skipped because it would incur another full-hour minimum and acceptance already
begins with the same three candidate-only qualification legs. The evidence-only memory protocol instead uses one
excluded warmup round followed by its paired measurement rounds.

The dedicated Linode shape is still a KVM guest. Two Sydney v1 placements
eventually recorded one steal tick during a timed leg: 1/1,317 ticks (0.075930%)
and 1/1,320 ticks (0.075758%). The frozen zero-steal rule correctly rejected
those blocks, and they are not reinterpreted retroactively. The accepted v2 fixed-throughput protocol retains
zero steal during each pre-leg idle screen and allows at most 0.1% steal during a timed leg. Its raw gate counters
remain in the evidence, and its zero-contaminant, 3% qualification-spread, ABBA/BAAB, bootstrap, verdict, and
publication gates are unchanged. The v2 memory config has separate paired-round, steal, evidence-completeness,
and unchanged-history gates.

The accepted v2 Sydney run qualified at 0.4999% pilot-median spread, kept all
timed legs at zero contaminants and at most 0.0742% steal, and measured the
candidate pooled median at 302,665.974 ns/unit versus 310,170.536 for the
baseline (-2.4195%). Its conservative nominal 95% interval was -3.3478% to
-1.5699%, so the predeclared +5% material-regression threshold was excluded.
The three actual final-candidate medians were 303,918.403, 303,335.093, and
300,979.767 ns/unit. Their raw sidecars and the complete comparison evidence
were measured on a rented Linode and were removed on 2026-09-13; the samples
remain in Git history.
This is acceptance evidence for that exact workload and host protocol, not a
cross-engine or blanket speed claim.

The accepted OOP run compared baseline `ae3beae2025feb21d0e37c70168204ae154ae125` with exact candidate
`f208c12d0ce266117743569d736346fae9a35db2` at fixed `n=24000`. Its pooled medians were 21,633.014 and 7,364.006
ns/unit, a -65.9594% candidate delta; the conservative nominal 95% interval was -66.3876% to -65.5482%, excluding
the predeclared zero-regression threshold. Three actual final-candidate rows and all raw evidence were measured
on a rented Linode and were removed on 2026-09-13; the samples remain in Git history. The complete candidate
includes both the
constant-field/method work and a weak-table GC-scan correction, so the result is not attributed to one opcode
family alone.

The completed evidence-only memory run compared the same baseline with exact candidate
`1370d6ccefba7a1956441247c3139af2a2cf31fa`. One ten-sample warmup round was excluded; 16 accepted rounds then
paired five scenarios across both builds for 160 unique fresh-process samples, each creating 512 VMs. Round 14's
first attempt observed one steal tick, discarded all ten samples, and passed on retry. Using anonymous
`after_run - program_ready` growth, the paired candidate-minus-baseline medians and nominal 95% intervals were:

- unloaded short minus empty: -32.0078 KiB/VM (-32.0156 to -32.0078);
- unloaded long minus empty: -258.0000 KiB/VM (-258.0000 to -258.0000);
- loaded short minus unloaded short: -105.3906 KiB/VM (-105.3984 to -105.3906);
- loaded long minus unloaded long: -260.9688 KiB/VM (-260.9766 to -260.9609).

The candidate residuals were approximately 0, 0, 0, and 7.0781 KiB/VM respectively. The exact samples, gate
records, manifest, and analysis are retained under
`evidence/vm-string-memory/1370d6ccefba7a1956441247c3139af2a2cf31fa/`; they intentionally do not appear in
throughput history, `RESULTS.md`, or `CLAIMS.md`.

The resulting evidence is Linux/x86_64-specific. macOS and Windows remain
supported build targets, but neither is an active benchmark target without a
dedicated measurement host.

The committed JSONL snapshot remains the summary source of truth. The historical
private matrix coordinator replaced it only with a complete single-revision Linux
cohort. Raw sidecars and provenance manifests live under
[`history/evidence/`](history/evidence/) and must not change that public schema.

List all configured engine slots:

```sh
cargo run -p script-bench-harness -- --list
```

Emit the same engine/workload inventory as deterministic JSON for wrappers,
CI, or generated-report integration:

```sh
cargo run -p script-bench-harness -- --list-json
```

Emit the wasm pre-lock gate tracker as deterministic JSON:

```sh
cargo run -p script-bench-harness -- --wasm-gate-json
```

Trusted CI reference command for the `call_ref` versus `br_table` wasm dispatch
gate across JavaScript runtimes (do not run locally):

```sh
node tools/run-wasm-dispatch-gate.js
```

Engine slots are tracked for `rustia`, `boa`, `koto`, `luaur`, `mlua-lua54`,
`mlua-luau`, `mlua-luau-jit`, `rhai`, `roto`, `ruau`, `rquickjs`, `wasmi`, and `wasmtime`. The
two mlua/Luau slots are the same adapter in explicit interpreter and native-JIT modes. Adapters are
feature-gated so heavyweight dependencies do not affect the normal workspace
build.

The matrix inventory can be listed without measuring:

```sh
cargo run -p script-bench-harness --features all-engines -- --list
```

Do not measure or publish the full matrix from a developer machine. The historical
private owned-host lane measured and published it as described under
[Cross-engine matrix](#cross-engine-matrix) below.

The coordinator verifies the frozen source and runtime artifacts, builds the
`all-engines` binary plus the separate Lua 5.4 binary, and measures the five
common workloads. It then runs the three wasm adapters on that same host under
Node/V8, the most widely used JavaScript runtime. Promotion requires the exact
65 native rows, 15 wasm rows, one dispatch run, Linux-only target identities,
and a single source revision.
The accepted four-runtime matrix is retained under `bench/wasm/history/` as
historical portability evidence; it is not repeated in the recurring public
benchmark. The matrix v4 host gate keeps every prewindow at zero steal. It also retains
per-repetition preparation, first-call, combined cold-start, and warmed steady-state timings. The
JIT row is published only when a matched JIT-off/JIT-on sentinel has the same checksum and shows at
least a 1.5x native execution signal. A timed group
passes at no more than 0.1% steal or one accounting tick; otherwise it restores
only that group's outputs and retries up to three times after a fresh quiet
prewindow, while retaining every rejected gate record. Each group shares one
five-minute quiet-acquisition deadline across its retries, and
`max_runtime_minutes`, checked at the phase boundaries as a hang detector,
remains the outer bound.
The generic `just benchmarks` local-publication path is retired so a macOS or
Windows developer machine cannot repopulate active results.

### Fixed-harness history reconstruction

Historical Rustia performance is reconstructed with
`rustia_history_backfill_v1`, not with each old commit's changing benchmark
code. The freezer selects first-parent commits that changed compiler, lexer,
parser, or VM source, pins the current harness and five Linux workloads by
SHA-256, and freezes one fixed `N` plus checksum per workload. The remote runner
overlays that exact harness onto every selected Rustia revision, builds all
binaries before timing, and measures the complete sequence inside one
quiet-host gate on the same dedicated Linode placement.

This skips documentation-, dashboard-, and orchestration-only commits: they
have the same engine binary and would add cost and visual noise rather than
historical information. Build failures or checksum changes fail the whole run
instead of silently changing the workload. Accepted output is immutable at
`bench/history/backfills/<harness-commit>.json`, with raw samples and provider,
host, build, protocol, and gate evidence under
`bench/evidence/history-backfill/<harness-commit>/`.

A reconstruction may run on the dedicated benchmark machine instead of an
ephemeral cloud instance. The artifact then records `host`, and the renderer
labels that series by machine. Reconstructions are comparable within themselves
and never across hosts, so each is charted as its own era; the artifact and the
evidence both name the pinned core, because core choice is unmeasured.

The published history is a single dedicated-machine series, and the chart shows
only it. Recorded matrix runs stay in `bench/history/runs.jsonl` and still
supply the 1x reference and the current-results tables, but they are measured on
a different host, so plotting them beside a reconstruction would invite a host
difference to be read as a performance change.

One command rebuilds the whole series on the owned host:

```sh
just bench-history-plan          # freeze and attest, measure nothing
just bench-history               # measure every revision and promote
just bench-history replace=1     # supersede an existing artifact
```

`replace=1` is the change-of-machine path. Numbers measured on a different box
are not comparable with the new one's, so upgrading or replacing the hardware
means re-deriving the series rather than appending to it, and the coordinator
refuses to overwrite an existing artifact without being told to.

The coordinator reaches the machine through `RUSTIA_BENCH_SSH`, an ssh
destination or `ssh_config` alias the operator keeps outside this repository.
No hostname, address or key path belongs here; `provider.host.name` is a
provenance label published in row data, never an ssh target. Before shipping
anything it attests the machine against the frozen config — CPU model, logical
CPU count, SMT off, free disk under the runner's home above the coordinator's
`MINIMUM_FREE_GIB` floor, that every pinned CPU really is in the kernel's
isolated set, and that each pinned core exposing a cpufreq policy is held at
its ceiling by it: the `performance` governor or a scaling range pinned to one
frequency, with boost off wherever the kernel exposes a knob for it. A pinned
core with no cpufreq policy at all is accepted, since nothing is scaling it, so
this checks the policy the kernel publishes rather than proving the frequency
the core ran at. A pinned core the scheduler still uses is the failure
this protocol exists to prevent, and it does not show up in the timings; nor
does one left to a dynamic governor, which can run a whole process at its
floor.

`sshDestination()` in `tools/run-dedicated-benchmark.js` makes "never an ssh
target" a check. Before any ssh or scp, on every command that reaches the host —
`host-check` and `unlock` included — it refuses a destination whose host part
equals a label this repository publishes: a runner class's `host` in
`operations-status.json`, or `provider.host.name` in any
`cloud/dedicated-*.json`, read afresh on every call. The host part is what is
left once an optional `ssh://` or `scp://` (the scp calls take the same
destination), a `user@`, a `:port`, a trailing `/` (ssh accepts a URI with an
empty path and still reaches the host), trailing dots (ssh drops one from a
URI's host) and IPv6 brackets are stripped, and case is ignored, since ssh
lowercases a host before it reads `ssh_config`. The refusal names the label and nothing else of the destination.
Only equality is refused, so choose an alias plainly unlike the label rather
than a variation of it. With `--local` the variable is never read.

The historical private profile kept reconstruction out of GitHub Actions through
`tools/check-workflows.js`. The source profile permits only hosted correctness
workflows; it does not enable reconstruction. Starting a new measurement requires
the trusted private benchmark service.

Only one reconstruction may run at a time. Concurrent runs sharing a worktree
and a target directory reset each other's checkouts mid-build and yield no
usable rows, so the runner takes an exclusive lock naming its pid and refuses to
start while a live one is held. A lock left by a killed run is taken over
automatically once its pid is gone; `just bench-history-unlock` clears the
history, matrix and memory lanes' locks by hand. A run wipes its working state
at startup but keeps the cargo target directory warm, which is what makes each
revision cost about 26 seconds rather than minutes. `just bench-host-check`
prints the host's free space, journal size, load average, driver state and each
isolated core's frequency policy — governor, range, boost and whether the
attestation would accept it — without touching anything; it is the first thing
to run when a lane refuses a quiet window twice in a row, or when a leg comes
back slow.

Revisions that predate the harness entry point need source changes to be
drivable at all: the workspace must declare the overlaid `bench` crate, and a
revision older than `Vm::call_function` needs that wrapper. Applying it is
allowed only when the wrapper is taken verbatim from the commit that introduced
it and exposes machinery the revision already contains, so the measured work
stays the revision's own. Every such change is listed in the artifact's
`source_modifications`, and the revision's workload checksums must match the
rest of the series exactly.

### Per-commit series

This section records the historical private coordinator. Its workflow is absent
from this source profile; the preserved series is historical evidence.

Every commit that lands on `main` starts a run that measures it on the owned
host under exactly the reconstruction's protocol, so the chart continues rather
than restarting. The weekly matrix's publication merge is no exception, now
that it is made with an app token and so starts push workflows as any person's
merge would: that run builds the engine, finds a byte-identical binary and
records nothing. It is the same exact gate that keeps a documentation commit
out of the series, and it costs one build on the host rather than a
measurement.
The historical private `.github/workflows/main-bench.yml` ran on the qualified runner class and reached
measurement only through `run-dedicated-benchmark.js`, which attests the machine
before it builds — including when it runs on that machine, where local mode
skips the transport but not the attestation.

The rows live in the published data repository, not here: they grow with every
commit, which is the same split the memory lane already uses. This repository
holds the protocol at `bench/history/series/ci-bench-main.json`, and
`tools/check-benchmark-series.js` validates the rows against it before anything
is pushed. The load-bearing rule is checksum equality — a changed checksum means
the workload changed, so the points either side are not measuring the same thing.
The per-commit coordinator also freezes its harness commit, fixed `N` values,
repetition count and expected checksums from that manifest. It must not inherit
those fields from the latest acceptance report, because that report may belong
to a newer benchmark era even while the live series is still extending the old
one. A reconstruction's targets are ancestors of its harness; a live-series
target is instead a descendant of the fixed harness. Frozen configs record that
direction explicitly, and the host verifies the declared ancestry before it
builds anything.

Runs wait for the host in the order they started. The four lanes that hold it
for a full measurement share one concurrency group declared with `queue: max`,
because a group without it keeps a single pending run and cancels that one when
another arrives. This lane depends on the order: rows are appended as runs
execute, the chart draws them in file order, and the unchanged-binary gate
compares against the last row. A run that fails records nothing for its
commit, and nothing measures that commit later. Re-run it before a later commit
records a row, after which the ancestry check refuses the append. The reasoning
is in [`design/81-benchmark-operations.md`](https://github.com/slowchop/rustia/blob/main/design/81-benchmark-operations.md#waiting-for-the-host).

Each source workflow commits its publication onto a fresh clone of the data
repository's `main` and hands it to `tools/push-benchmark-data-publication.js`,
which pushes that commit to `main` as a plain fast-forward. The data
repository's branch ruleset keeps `main` pull-request-only for people and
lists deploy keys as its bypass, so the writer key is the only thing that
lands there directly. When another publication has landed since the clone, the
helper fetches, rebases the single commit and pushes again, a bounded number
of times; a rebase conflict aborts and fails the source workflow, which a
re-run rebuilds from the current data. Nothing in the data repository runs
on GitHub-hosted compute: the earlier `publication/*` branch plus
merge-workflow scheme needed a hosted runner there, and the source workflow
waited on it.

A commit that builds a byte-identical binary records nothing. The gate is the
compiled binary's hash rather than a path heuristic, so documentation and tooling
commits do not add duplicate points.

#### Host updates

The host installs its own operating-system security updates, kernel included,
at 05:20 Hobart time and restarts for them in a slot after that, both only while
no benchmark is running. A new kernel, libc, linker or microcode can move every
number after it, and the series alone cannot show that: its next point is a new
commit, so a host step and a code step look alike.

So every series row carries `kernel` and `host_fingerprint`, a hash over the
running kernel, the CPU microcode and the installed package set
(`run-dedicated-benchmark.js host-fingerprint`). Before measuring anything, the
lane compares the host's fingerprint with the last one recorded. When it
differs, it first measures the last recorded commit again (`main --commit`)
and `tools/bench-host-control.js` compares that control with the commit's own
rows: same code, same protocol, different host software. The result is
appended to `bench/history/series/ci-bench-host-controls.jsonl` in the data
repository whether or not anything moved, so the spread between controls is
measured rather than assumed. A workload that moved beyond its calibrated
same-host threshold from `bench/pr-screening.json`, in either direction, fails
the run after publishing. A person then decides whether to re-measure the
history on the updated host (`just bench-history replace=1`, about 35 minutes).
Nothing re-measures or rewrites rows by itself yet: that waits until enough
controls exist to set a threshold for it.

The data step marks successful publication through a job output. A dependent
call to the sole dashboard publisher then renders the saved rows and controls.
The dependency uses `always()` and requires that output: a retained host-control
finding still reaches the graph when the separate drift step fails, while a
failed data push cannot trigger publication. This closes the race where a
push-triggered dashboard job completed before the measurement did.

The published graph includes the control ledger and breaks the line at flagged
controls. Its change table compares only points within each displayed host
period. Historical source dates still position reconstructed points; actual
measurement times determine their host period. A control compares against the
source revision's original row, not the previous control, so repeated findings
must not be added together. The graph reports observed ratios to a separately
measured reference, not code-only improvements or causal package diagnoses.
Raw numbers are preserved. An unchanged fingerprint today does not clear earlier
warnings. See [the integrity investigation](https://github.com/slowchop/rustia/blob/main/design/99-benchmark-history-integrity.md).

Host steps a person looked at and left in the series:

| Host change | Control | Moved | Decision |
| --- | --- | --- | --- |
| Kernel `6.8.0-138` to `6.8.0-146` plus a package upgrade, 2026-10-01 | `911bad60`, byte-identical binary, 2026-10-02 | `sparse_tables` +3.5% median (threshold 1.2%); the other four within threshold (`closures_upvalues` -3.3%, `generic_iteration` +2.3%, `stdlib_math_table` +0.2%, `dense_tables` 0.0%) | Kept. Series points from 2026-10-01 on read `sparse_tables` about 3.5% slower than the points before them for host reasons, not code |

That private workflow was also scheduled daily at 21:00 UTC, after the host's restart window, so
an update is controlled the morning it lands, not at the next merge. Those
runs check the host only and never measure HEAD. After an update the same
commit can build a different binary (a new linker), so the byte-identical gate
would no longer stop a commit being recorded twice. For the same reason, a push
run never measures a HEAD that is already in the series.

The renderer joins the live series to a reconstruction only when host, pinned
core, harness, workloads, fixed `N` and every checksum agree; otherwise it draws
a separate labelled line. Two protocols on one line is the confound that moving
to a single host removed.

Publication is not dispatched per commit. Rows accumulate and the chart refreshes
on the next publication run, which keeps a twenty-minute job and a Pages commit
off every merge.

Where two revisions compile to byte-identical binaries, their independently
measured timings bound this host's run-to-run variation. That check is free and
belongs in the evidence.

### Pull-request screening

The historical private `.github/workflows/pr-bench.yml` measured a pull request
against its merge base with the target branch and reported the result in the job
summary. It is absent from this source profile. It
records nothing: no row, no published artifact, and no write access to either
repository. The merge base rather than main's tip, because an unrelated commit
landing mid-review would otherwise be attributed to the pull request.

Two passes measure both sides, with the order reversed in the second, so the
sequence is A B B A and drift across the session lands on both sides instead of
on whichever went second. Each pass is one frozen `--pair` config through the
same runner the reconstruction uses; the lane adds no second measurement path.

Three cases end the run early, each with a reason rather than a number:

- the merge base is this commit, so there is nothing to compare against;
- the change edits a file the frozen config pins by hash — the five workload
  files, `bench/src/main.rs`, `bench/Cargo.toml`, `Cargo.lock`,
  `rust-toolchain.toml`, or `bench/current-results.json`, which supplies the
  fixed sizes and expected checksums. The run would otherwise fail on a checksum
  deep inside the host, which says nothing useful. That list comes from the
  freezer itself rather than being restated here: the first version named the
  `bench/` directory, and refused the lane's first pull request — a
  documentation change — as a protocol change;
- the change builds a byte-identical binary to its merge base.

**Thresholds are measured, not chosen.** `bench/pr-screening.json` comes from
`just bench-calibrate`, which measures one revision four times and compares it
against itself under every balanced split of those passes — both sides of every
split are the same binary, so each interval is this host's own spread. A
workload's threshold is the widest end of the widest split, rounded up. There is
no default and no fallback: a guessed figure that happens to sit inside the
host's spread reports movement on changes that did nothing, which is worse than
having no lane. The screening lane refuses to run until a calibration exists.

One split is one draw. Two calibrations of the same commit produced thresholds
that disagreed by a factor of eight on `generic_iteration`, which is what a
two-leg hierarchical bootstrap does with two legs to resample from. Every
workload also reports `widened_by: leg_variation`, so more repetitions per leg
would tighten nothing here — more passes would.

The calibration is also the protocol's own canary, and it caught something on
its first real run. Four passes admit three balanced splits; all three are
analysed. Under `1,3 vs 2,4` — every candidate pass immediately after its
baseline — `sparse_tables` read as 1.2% *faster than itself*, with an interval
of [-1.71, -0.66] that excludes zero. A later run reproduced it on a different
workload: `stdlib_math_table`, same split, this time slower.

That is not a fault in the code. It is monotonic drift across a measurement
session: later passes are faster, and that arrangement does not cancel it. The
screening lane does not measure that way — two passes ordered A B then B A put
each side at positions 1 and 4 or 2 and 3, which straddles the run — so the
canary fails only on the straddling split, which is the arrangement whose
indistinguishability is actually being claimed.

The other two splits keep their job. They are the same binary arranged so drift
does not cancel, so whatever they show is movement this host can produce: they
widen the threshold, and each workload records which of them read a change in
`drift_splits`. This is what turns A B B A from a stated precaution into a
measured requirement — a non-straddling arrangement of the same passes reports
a change that did not happen.

Recalibrate after any change to the machine — kernel, firmware, isolation, or
the box itself. Adopting a calibration is a deliberate commit, since it changes
what every later pull request is judged against.

Screening reads five workloads at once, so the interval level matters: five
independent 95% readings give roughly a 23% chance that one of them "moves" on a
change that did nothing. More repetitions do not fix that — they narrow each
interval without touching the per-test error rate — so the lane screens at 99%
instead, which costs nothing in wall clock.

A screening result is a reading for a reviewer, not a measurement the project
stands behind. The acceptance protocol produces those, and it runs far longer.

The lane was verified end to end by a documentation-only pull request: it built
both revisions, found byte-identical binaries, skipped the second pass and
invoked the analyzer zero times, which is the correct outcome for a change that
cannot have moved anything. Two defects surfaced only by running it — a `git
fetch` in a job that drops its credentials at checkout, and a protocol-change
rule that named the `bench/` directory and so declined the pull request for
touching the README.

One inefficiency is left in deliberately: the short-circuit fires after the
first pass, which has already measured both sides, because the runner builds and
measures in one invocation. Discovering "no engine change" therefore costs about
ninety seconds more than it needs to. Threading a build-only mode through the
runner to save that would add a second path through the thing whose single path
is the reason the two lanes cannot drift.

Freeze only from a clean committed checkout and commit the generated config;
`just bench-history` does the freeze itself and reuses an already-frozen config
for the same harness commit rather than silently re-deriving one.

`provider.host.max_runtime_minutes` bounds a run. On a rented instance that
bound was a bill; on an owned machine it is a hang detector, checked at each
revision boundary so a wedged build fails in seconds rather than after another
twenty minutes. Historical source commit time is retained separately from
honest measurement time; renderers order the reconstructed trend by the frozen
target list rather than pretending the measurements happened in the past.

The historical private workflow policy allowed measurement only on `bench-solo`
through the attesting entrypoint `tools/run-dedicated-benchmark.js`; the shared
runner pool validated committed artifacts. The source profile's
`tools/check-workflows.js` instead permits only hosted correctness CI, with no
measurement workflow.

`all-engines` intentionally excludes `mlua-lua54` because `mlua-lua54` and
`mlua-luau` cannot be linked into one binary. The trusted CI reference command
for the separate Lua 5.4 mlua adapter is (do not run locally):

```sh
cargo run -p script-bench-harness --no-default-features --features mlua-lua54 -- --engines mlua-lua54
```

In this slice, `rustia` works through the default `rustia-engine` feature,
`boa` works with `--features boa`, `koto` works with `--features koto`, Luaur
works with `--features luaur-engine`, Lua 5.4 works with `--features mlua-lua54`,
the reference Luau interpreter comparison works with `--features mlua-luau`, its optional native
code generator works with `--features mlua-luau-jit`, Rhai works
with `--features rhai-engine`, Roto works with `--features roto`, Ruau works
with `--features ruau-engine`, QuickJS works with `--features rquickjs-engine`, Wasmi
works with `--features wasmi-engine`, and Wasmtime works with
`--features wasmtime-engine`. The crate also builds with `--no-default-features`
so the standalone harness shell is not tied to rustia internals.

### Cross-engine matrix

The historical private `.github/workflows/weekly-matrix-bench.yml` measured all
thirteen engines across the five workloads on the owned host and landed the
accepted cohort on `main`. That workflow and its contents-write grant are absent
from this source profile. In the private coordinator, the cohort
belongs in source history, unlike the per-commit series, whose rows grow forever
and live in the data repository. `main` accepts changes only through pull
requests — an organisation rule covering every active private repository, which
no workflow token can bypass — so the
publication commit goes to a `matrix-publish/<commit>` branch and
`tools/land-matrix-publication.js` opens the pull request and merges it with a
merge commit. The operations status, package and publishability inventories,
and current-results report in that commit all describe the `main` it was built
on, so the helper merges only the commit the run pushed, and only while `main`
is still that commit — not merely while the branch merges cleanly. If `main`
moved meanwhile, the publication is rebuilt on top of it and landed again,
three attempts at most. A refusal that rebuilding cannot fix — a repository
rule, a missing permission — fails the run at once with GitHub's reason.

Nothing runs on that pull request: the publication commit carries `[skip ci]`,
so GitHub creates no pull-request run for it. The marker predates the app
identity below — runs on a pull request opened with the workflow token were
held until someone with write access approved them, and the helper merges
seconds after opening it, so the first landing (#40, `aaa71be`) left three
failed runs with no jobs behind it. The pull request title does not carry the
marker, because the title becomes the body of the merge commit on `main`.

The merge itself is made as the `slowchop-ops-controller` app rather than with
the workflow token. A named app can be named as a ruleset bypass actor and
`github-actions[bot]` cannot, so this is what lets `main` require status checks
even though the publication commit is never checked. The workflow mints an
installation token after the measurement — such a token lives one hour and a
cohort takes about forty minutes — and scopes it to the landing helper alone;
the dispatches keep the workflow token, because the app holds `actions: read`
and dispatching a workflow needs write.

A merge made with an app token does start push workflows, unlike one made with
the workflow token, so the explicit dispatches of the dashboard publication and
`ci.yml` on `main` may duplicate runs the merge already created. Both tolerate
that: CI on `main` shares one concurrency group, and the dashboard publication
commits only when its render changes. They are kept until a landing has shown
which runs the merge creates by itself. A dispatched run reports on `main`'s new
tip but gates nothing, and since a bot started it, a failure notifies no one: it
shows only as a red check on that commit. The per-commit series is not
dispatched, because a publication changes no engine code. Each dispatch is
attempted whatever happens to the other, and a failed one fails the run with the
name of the workflow to dispatch by hand; re-running the job would measure and
land another cohort. Before committing, the step also runs the two
release-preflight checkers whose result a cohort can change and that it did not
otherwise run — the wasm results and the project status — so a cohort that would
turn `main` red is not landed.

Two constraints follow. Required status checks reach a publication pull request
like any other, and it has none to show, so the app's bypass is load-bearing:
withdraw it and every cohort stops landing. At the documented private landing,
`main` required eleven checks through a repository ruleset whose only bypass
actor was that app. A publication pull request landed only as a merge commit,
which was the only merge that private repository offered, squash and rebase
being disabled. These settings do not describe the new source repository. In
that coordinator, a pull request left open by a failed run should be closed, not merged by
hand: a hand merge does not check that `main` is still the commit it was
rendered on, and the squash and rebase buttons that would copy the marker onto
`main` are off for that reason.

It replaced a nightly job that created a billed cloud instance whenever `main`
had gained performance-relevant commits since the last cohort. Both properties
are gone. The gate asked the Rustia-progress question, which the per-commit
series now answers directly; this lane asks about competitors, and running it
unconditionally also produces a host-drift signal — kernel, microcode, thermal —
that a gated run would hide. Its cost is forty minutes of an otherwise idle
owned machine rather than a bill. That private workflow was scheduled every Sunday at 15:00 UTC, an hour
the host is idle, and can be dispatched by hand for an out-of-cycle cohort; the
schedule was added once the first owned-host cohort had landed and been looked
at.

The runner is the same one the rented instance used, with the assumptions that
belonged to a throwaway machine removed: it no longer requires root, no longer
installs into `/root`, no longer permanently masks the host's package timers,
wipes only its own working state between runs while keeping both cargo target
directories warm, and takes an exclusive lock. `max_runtime_minutes` used to be
a billing ceiling the coordinator enforced by destroying the machine; on an
owned host nothing destroys anything, so it is now a hang detector checked at
the phase boundaries.

The owned lifecycle has its own config schema rather than borrowing the rented
one's provider shape, and its steal ceilings are exactly zero. On bare metal
steal is structurally absent, so a non-zero reading is a fault rather than
something to tolerate. Host evidence records `uname -srvm` rather than
`uname -a`: on a throwaway instance the node name was a random string that died
with the machine, and on an owned host it is the real name of a real machine in
a file committed to a public repository. The coordinator checks the form rather
than looking for a name: the second field must be the kernel release the
attestation read, which is where `uname -a` puts the node name.

**Before the rented rows are deleted**, `tools/record-host-bridge.js` writes the
per-engine comparison between the last rented cohort and the first owned one.
Published figures are ratios against the reference measured in the same run, so
a change of host cancels there; absolute nanoseconds do not, and after the
deletion that comparison is unrecoverable.

## History Schema

Each line in `history/runs.jsonl` is a JSON object:

```json
{
  "timestamp_unix": 0,
  "commit": "8c0211d",
  "target": "linux-x86_64",
  "host": "ci-bench",
  "engine": "rustia",
  "dialect": "luau",
  "workload": "dense_tables",
  "unit": "build+sum+sort of 256-elem array",
  "n": 1000,
  "reps": 9,
  "ns_per_unit_min": 1.0,
  "ns_per_unit_median": 1.1,
  "units_per_sec": 1000000.0,
  "checksum": "number:123",
  "prepare_ns_min": 1000.0,
  "prepare_ns_median": 1100.0,
  "first_call_ns_min": 200.0,
  "first_call_ns_median": 220.0,
  "cold_start_ns_min": 1200.0,
  "cold_start_ns_median": 1320.0,
  "mode_family": "rustia",
  "execution_mode": "interpreter",
  "native_verified": false,
  "native_verification": "not-applicable",
  "native_verification_ratio": null
}
```

`host` names the machine the row was measured on. It is provenance published in
row data and nothing more: no tool resolves it, connects to it, or derives an
address from it. Runners pass it as `--host`, derived from the frozen config, so
a row cannot claim a machine the run was not qualified for.

The field exists because both tables merge cohorts — `current-results.json`
keeps the newest row per target, engine, and workload across every cohort in
history, and `memory/current-results.json` takes each engine's rows from the
newest cohort that measured it — so a partial re-measurement refreshes what it
covered and leaves the rest in place. Once those others came from a different
machine, the table was a mix and nothing said so. A published table is therefore
required to be one machine's readings: `current-results.json` carries
`metadata.hosts`, `tools/check-benchmark-results.js` requires that list to hold
exactly the one host its rows name and to match the qualified timing authority in
`operations-status.json`, and `render-current-results.js` refuses to render a
table whose rows disagree rather than labelling the mix — every figure on the
page is a ratio between two rows, and an annotated cross-host ratio is still a
wrong number. The same check covers `memory/current-results.json`; it requires
every cohort in `memory/history/runs.jsonl` to be one machine and pins the
latest cohort — the one the page draws — to the authority, while older cohorts
may name the machine they were measured on. The renderer pins the same three
things itself, because the publication lane installs the data repository's
memory table and history after the checker has run. The history charts apply
the rule from the row data: the recorded timing chart draws only rows measured
on the machine the mlua/JIT reference was measured on and counts the rest as
runs not drawn, a reconstruction or live series from another machine is
refused, and the memory history draws one machine — the latest cohort's —
against a mlua/JIT reference measured on that same machine, saying how many
cohorts from another machine, or from before hosts were recorded, it leaves
undrawn. The label itself may contain only `[A-Za-z0-9_-]{1,128}`: no dots, no
colons, so it can never be a fully qualified name or an address. Every file
that emits or validates one carries that rule as the one literal
`const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;`, and the checker's self-test
reads each and refuses to run if any has drifted:
`tools/check-benchmark-results.js`,
`tools/render-benchmark-operations-status.js`,
`bench/render-current-results.js`, `bench/analyze-memory-samples.js`,
`bench/run-memory-cohort.js`, `bench/publish-memory-analysis.js`,
`tools/merge-rustia-ci-memory-data.js`, `tools/check-dedicated-config.js`,
`tools/run-linode-benchmark.js`, `tools/run-linux-matrix-benchmark.js`,
`tools/run-linux-memory-benchmark.js`,
`tools/run-linux-history-backfill.js` and `tools/record-host-bridge.js`. The
harness's `--host` check in
`bench/src/main.rs` is read alongside them for the same character set and
128-character cap.

Rows written before the field existed do not carry it, and the harness reads it
as optional for that reason: the absence means "measured before hosts were
recorded", not "measured somewhere unnamed". A row that carries the key must
carry a non-empty value, and `metadata.hosts` is written only when every row
names its machine. The committed history names a host on every row.

The future website should treat the versioned JSONL snapshot as the summary
source of truth and generate charts from `engine × target × workload`
series.

Raw repetitions are intentionally excluded from this public history schema, while their derived
minimums and medians are retained. `--samples-json <path>` exports the ordered steady-state,
preparation, first-call, and combined cold-start samples without changing JSONL history records or
generated current-results artifacts. A sidecar is optional for ordinary trusted
CI calibrated runs, but required when a fixed-N run appends history; accepted
milestone evidence is retained under `history/evidence/<full-commit>/`.

`--list-json` emits a separate schema-identified metadata document for the
available engine slots, feature gates, workload IDs, default workload set, and
result paths. It is intended for tooling; retained native benchmark measurements
stay in the JSONL snapshot above. In that document, `implemented` means the
source adapter exists, while `available` means this specific binary was compiled
with the adapter's feature enabled.

Validate committed history without running benchmarks:

```sh
cargo run -p script-bench-harness -- --check-history
```

The check validates required fields, known engine/workload IDs, dialect and unit
consistency, positive finite timing values, clean commit IDs, and monotonic
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
with best-first ranked bars, workload heatmaps, and the controlled-cohort history
ledger, plus [`CLAIMS.md`](CLAIMS.md),
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

## Static Report

`current-results.html` is the single generated report for target-first
implementation and language summaries, workload detail, protocol-aware history,
Linux-hosted wasm results, and claim-gate context. Its display toggle switches
the headline and heatmaps between absolute timing and speed relative to rustia.

```sh
python3 -m http.server 8000 --bind 0.0.0.0
```

Open `http://HOST:8000/bench/current-results.html` over the trusted VPN.

The earlier alternatives remain under [`chart-mockups/`](chart-mockups/) as
design references; the ranked bars, heatmap, and cohort-ledger approach is now
the canonical report.

## Grabby application build results

The same site now renders a separate `#grabby-builds` section from
[`grabby-builds/results.json`](grabby-builds/results.json), selected from the two
independently reviewed Grabby ci-bench experiments. It is not a Rustia workload,
engine score or historical-series row. Both original source-report commits and
digests are retained; full reports and private machine diagnostics are not copied.

The two LTO-off controls belong to different sequential runs. Preserve both;
do not multiply paired speedups or call this one three-arm comparison. Normal
development keeps full debugging; quick mode trades compiler debugger detail for
faster compilation, and measured development runtime costs remain visible.

Run `node tools/check-grabby-build-results.js` to validate the selected schema,
pair arithmetic, public-field allowlist and publication routing, then regenerate
with `node bench/render-current-results.js`. These commands validate/render
existing evidence and take no performance measurements. New results require
fresh ci-bench evidence and independent source review before updating this data.

The historical private main-only publisher mirrored the selected JSON to the data
repository and published it beside `index.html` as `grabby-builds.json`. This source
profile has no publication workflow. That private publisher was the sole Pages
writer, including after memory cohorts; it used main rendering code plus the
data repository's retained live series and memory. Do not hand-edit
the generated page or publish from a historical measurement checkout.

The non-default `string_opcodes` workload is Rustia-only supplemental acceptance.
It uses a top-level program to exercise all nine specialized string opcodes;
nested functions intentionally use ordinary builtin calls. The `strings` mode
of the acceptance workflow pins its source, fixed work and paired order, retains
raw samples, and reports intervals without changing the five-workload calibration.
