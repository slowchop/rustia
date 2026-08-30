# script-bench-harness

`script-bench-harness` is a standalone benchmark harness for embedded scripting
engines. It lives in the rustia repo while rustia is early, but it is structured
to split into its own OSS project with a website, published result history, and
stats charts.

Split notes are tracked in [`STANDALONE.md`](STANDALONE.md).

The harness writes one JSONL record per completed engine/workload measurement,
so history can be committed, served statically, or uploaded by CI without needing
a hosted metrics service.

The active performance product is deliberately narrower than Rustia's build
support: native measurements are `linux-x86_64` on an ephemeral dedicated
Linode, and wasm runtimes are measured on that same Linux host. macOS and
Windows timings are excluded until dedicated machines exist for them.

The default build runs the in-repo `rustia` adapter over five workload families:

- `dense_tables`
- `sparse_tables`
- `closures_upvalues`
- `generic_iteration`
- `stdlib_math_table`

The inventory currently contains 19 workload entries. Non-default workloads are
kept out of the committed timing snapshot until they have intentional
measurement history; `string_building` is wired across every requested adapter
as the #15 string/concat workload source set. Entries #16–#18 are portable
burst/churn/intentional-retention memory probes, and #19–#21 are deterministic
steady/history/intentional-retention todo-app memory workloads for Luau and
Koto.

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

The JSONL summary stores only the stable min/median values. Direct harness runs
may append rows, while the dedicated Linode matrix coordinator atomically
publishes a fresh, complete Linux cohort. For local analysis, write the raw
per-repetition `ns/unit` values to a separate JSON sidecar:

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

The local hardening diagnostics, 2026-07-20 accepted dense/OOP comparisons, and completed string-memory evidence are
recorded in [`design/56-performance-research.md`](https://github.com/slowchop/rustia/blob/main/design/56-performance-research.md),
including exact identities, paired protocols, confidence intervals, retained artifacts, and the deliberately
conservative claim posture.

## Memory benchmark direction

The existing `vm-string-memory` sampler is a focused Rustia A/B evidence tool, not a general engine comparison.
The first-class memory product is specified in
[`design/57-memory-benchmarking.md`](https://github.com/slowchop/rustia/blob/main/design/57-memory-benchmarking.md). Its child/supervisor, cgroup-v2
sampling, repeated-epoch analyzer, synthetic controls, deterministic todo workloads, separate history, and
pending/ready dashboard integration are now implemented; the first dedicated-Linux pre-reclamation cohort is
published as `pre-reclamation-18b58d8` (110 rows, 990 fresh processes). Throughput and instrumented memory
measurements remain separate.

```sh
just bench-memory-self-test
just bench-linode-memory-plan bench/cloud/linode-cross-engine-memory-COMMIT.json
just bench-linode-memory bench/cloud/linode-cross-engine-memory-COMMIT.json
```

The billed command uses the same fail-closed Linode lifecycle as timing: it downloads the complete raw cohort,
confirms deletion, independently reruns the analysis locally, then atomically installs evidence, memory history,
the current snapshot, and the generated dashboard.

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

The current accepted DigitalOcean VM Core 2 C2.2 comparison is frozen in
[`cloud/digitalocean-core2-84e732a-ams3.json`](cloud/digitalocean-core2-84e732a-ams3.json).
It compares exact baseline `9695a62` with candidate `84e732a` on
`generic_iteration` using 101-repetition sample blocks in the fixed-work paired
protocol and bounded idle/steal gates described below. Its exact AMS3 Basic
Intel 4-vCPU/8-GiB shape is frozen in the config; the plan fails instead of
substituting a different size or region. The accepted eight-leg comparison
measured 8.1651% lower candidate ns/unit with a conservative nominal 95%
interval of -10.2126% to -5.9577%. Evidence is retained under
[`history/evidence/84e732add4863150bd993f3354b274ab9c3ee41a/`](history/evidence/84e732add4863150bd993f3354b274ab9c3ee41a/).

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
fresh-process rounds and never appends throughput history. A no-argument Linode command continues to select the
dense config. Select another frozen protocol explicitly and use the same path for planning and execution:

```bash
just bench-linode-plan bench/cloud/linode-oop-f208c12.json
just bench-linode bench/cloud/linode-oop-f208c12.json
just bench-linode-plan bench/cloud/linode-vm-string-memory-1370d6c.json
just bench-linode bench/cloud/linode-vm-string-memory-1370d6c.json
```

The DigitalOcean, Vultr and Scaleway coordinators that ran these comparisons
have been deleted. Timing now happens on one owned, attested host, so nothing
in this repository authenticates to, creates, or pays for a cloud instance --
and the operating instructions for those providers, which named a developer
machine by host alias, are gone with them.

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

Create a short-lived personal access token with only **Linodes: Read/Write** and
**Account: Read Only**. Account read access lets the coordinator reject an
account-wide automatic-backup default before creating a billable instance. The
token is entered through a private TTY prompt and stored mode `0600` only under
the ignored `.rustia-benchmark-state/linode/` directory:

```bash
just bench-linode-auth
just bench-linode-plan
just bench-linode
```

Run these from the operator's own checkout. Earlier revisions of this file
spelled them as `ssh` invocations against a named developer machine; a host
alias is infra-identifying and does not belong in a public repository, and the
recipes never needed it -- `-t` for the two that prompt for a token is the only
part that mattered.

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

Crash recovery is provider-isolated:

```bash
just bench-linode-status
just bench-linode-unlock
just bench-linode-destroy
just bench-linode-destroy RUN_ID
just bench-linode-promote RUN_ID       # offline retry after a completed local promotion rolls back
```

The promotion retry never reads the API token or contacts Linode. It requires a confirmed-deleted, completed run
and rechecks the journaled archive, original Git identities, raw samples, and every host gate. Fixed-throughput
runs additionally require the qualification spread, recomputed bootstrap verdict, exact final-history rows, and
generated result files. Evidence-only memory runs require `evidence_complete`, recompute the paired analysis, and
prove that throughput history remains unchanged.

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
300,979.767 ns/unit; their raw sidecars and the complete comparison evidence are
retained under `history/evidence/ed65077c2e779ec6a641b8ca2f8c8258b2508a2d/`.
This is acceptance evidence for that exact workload and host protocol, not a
cross-engine or blanket speed claim.

The accepted OOP run compared baseline `ae3beae2025feb21d0e37c70168204ae154ae125` with exact candidate
`f208c12d0ce266117743569d736346fae9a35db2` at fixed `n=24000`. Its pooled medians were 21,633.014 and 7,364.006
ns/unit, a -65.9594% candidate delta; the conservative nominal 95% interval was -66.3876% to -65.5482%, excluding
the predeclared zero-regression threshold. Three actual final-candidate rows and all raw evidence are retained
under `history/evidence/f208c12d0ce266117743569d736346fae9a35db2/`. The complete candidate includes both the
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

The committed JSONL snapshot remains the summary source of truth. The active
matrix coordinator replaces it only with a complete single-revision Linux
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

Measure the `call_ref` versus `br_table` wasm dispatch gate across available
JavaScript runtimes:

```sh
node tools/run-wasm-dispatch-gate.js
```

Engine slots are tracked for `rustia`, `boa`, `koto`, `luaur`, `mlua-lua54`,
`mlua-luau`, `mlua-luau-jit`, `rhai`, `roto`, `ruau`, `rquickjs`, `wasmi`, and `wasmtime`. The
two mlua/Luau slots are the same adapter in explicit interpreter and native-JIT modes. Adapters are
feature-gated so heavyweight dependencies do not affect the normal workspace
build.

The matrix switch is present now:

```sh
cargo run -p script-bench-harness --features all-engines -- --list
```

Do not publish the full matrix from a developer machine. Plan and run it through
the explicit dedicated-Linode lifecycle with a committed frozen config:

```sh
just bench-linode-matrix-plan bench/cloud/LINODE_MATRIX_CONFIG.json
just bench-linode-matrix bench/cloud/LINODE_MATRIX_CONFIG.json
```

The coordinator verifies the frozen source and runtime artifacts, provisions the
declared Linux/x86_64 shape, builds the `all-engines` binary plus the separate
Lua 5.4 binary, and measures the five common workloads. It then runs the three
wasm adapters on Node/V8, the most widely used JavaScript runtime, on that same
host. Promotion requires the exact 65 native rows, 15 wasm rows, one dispatch run, Linux-only
target identities, a single source revision, and confirmed instance deletion.
The accepted four-runtime matrix is retained under `bench/wasm/history/` as
historical portability evidence; it is not repeated in the recurring public
benchmark. The matrix v4 host gate keeps every prewindow at zero steal. It also retains
per-repetition preparation, first-call, combined cold-start, and warmed steady-state timings. The
JIT row is published only when a matched JIT-off/JIT-on sentinel has the same checksum and shows at
least a 1.5x native execution signal. A timed group
passes at no more than 0.1% steal or one accounting tick; otherwise it restores
only that group's outputs and retries up to three times after a fresh quiet
prewindow, while retaining every rejected gate record. Each group shares one
five-minute quiet-acquisition deadline across its retries, and the existing
55-minute Linode lifecycle remains the outer bound.
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
CPU count, SMT off, and that every pinned CPU really is in the kernel's isolated
set. A pinned core the scheduler still uses is the failure this protocol exists
to prevent, and it does not show up in the timings.

Measurement stays out of GitHub Actions: `tools/check-workflows.js` forbids the
harness there, and starting a run remains an explicit operator action.

Only one reconstruction may run at a time. Concurrent runs sharing a worktree
and a target directory reset each other's checkouts mid-build and yield no
usable rows, so the runner takes an exclusive lock naming its pid and refuses to
start while a live one is held. A lock left by a killed run is taken over
automatically once its pid is gone; `just bench-history-unlock` clears one by
hand. A run wipes its working state at startup but keeps the cargo target
directory warm, which is what makes each revision cost about 26 seconds rather
than minutes.

Revisions that predate the harness entry point need source changes to be
drivable at all: the workspace must declare the overlaid `bench` crate, and a
revision older than `Vm::call_function` needs that wrapper. Applying it is
allowed only when the wrapper is taken verbatim from the commit that introduced
it and exposes machinery the revision already contains, so the measured work
stays the revision's own. Every such change is listed in the artifact's
`source_modifications`, and the revision's workload checksums must match the
rest of the series exactly.

### Per-commit series

Every commit that lands on `main` is measured on the owned host under exactly
the reconstruction's protocol, so the chart continues rather than restarting.
`.github/workflows/main-bench.yml` runs on the qualified runner class and reaches
measurement only through `run-dedicated-benchmark.js`, which attests the machine
before it builds — including when it runs on that machine, where local mode
skips the transport but not the attestation.

The rows live in the published data repository, not here: they grow with every
commit, which is the same split the memory lane already uses. This repository
holds the protocol at `bench/history/series/ci-bench-main.json`, and
`tools/check-benchmark-series.js` validates the rows against it before anything
is pushed. The load-bearing rule is checksum equality — a changed checksum means
the workload changed, so the points either side are not measuring the same thing.

A commit that builds a byte-identical binary records nothing. The gate is the
compiled binary's hash rather than a path heuristic, so documentation and tooling
commits do not add duplicate points.

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

`.github/workflows/pr-bench.yml` measures a pull request against its merge base
with the target branch and reports the result in the job summary. It
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

Hosted CI only validates the committed artifacts. `tools/check-workflows.js`
forbids fresh native or wasm measurement commands in GitHub Actions; starting a
billed Linode remains an explicit operator action.

`all-engines` intentionally excludes `mlua-lua54` because `mlua-lua54` and
`mlua-luau` cannot be linked into one binary. Run the Lua 5.4 mlua adapter as a
separate binary:

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

`.github/workflows/weekly-matrix-bench.yml` measures all thirteen engines across
the five workloads on the owned host and pushes the accepted cohort to `main`.
It is this repository's single sanctioned contents-write grant: the cohort
belongs in source history, unlike the per-commit series, whose rows grow forever
and live in the data repository.

It replaced a nightly job that created a billed cloud instance whenever `main`
had gained performance-relevant commits since the last cohort. Both properties
are gone. The gate asked the Rustia-progress question, which the per-commit
series now answers directly; this lane asks about competitors, and running it
unconditionally also produces a host-drift signal — kernel, microcode, thermal —
that a gated run would hide. Its cost is forty minutes of an otherwise idle
owned machine rather than a bill.

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
a file committed to a public repository.

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

The future website should treat the versioned JSONL snapshot as the summary
source of truth and generate charts from `engine × target × workload`
series.

Raw repetitions are intentionally excluded from this public history schema, while their derived
minimums and medians are retained. `--samples-json <path>` exports the ordered steady-state,
preparation, first-call, and combined cold-start samples without changing JSONL history records or
generated current-results artifacts. A sidecar is optional for ordinary local
or calibrated runs, but required when a fixed-N run appends history; accepted
milestone evidence is retained under `history/evidence/<full-commit>/`.

`--list-json` emits a separate schema-identified metadata document for the
available engine slots, feature gates, workload IDs, default workload set, and
result paths. It is intended for tooling; active native benchmark measurements
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
