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

The inventory currently contains 18 workload entries. Non-default workloads are
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

The current DigitalOcean Region Engine v1 follow-up is frozen in
[`cloud/digitalocean-region-b861ee9.json`](cloud/digitalocean-region-b861ee9.json).
It compares exact pre-region baseline `7a69d1c` with candidate `b861ee9` on
`generic_iteration` using 101-repetition sample blocks in the same fixed-work
paired protocol and strict
idle/steal gates described below. Its exact Basic Intel 4-vCPU/8-GiB shape in
Amsterdam replaces rejected, fully cleaned-up Sydney, Frankfurt, New York, and
San Francisco placements;
the plan fails instead of substituting a different size or region. The earlier dense-table follow-up for baseline
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

Two tracked diagnostic profiles compare useful shared-CPU alternatives:
regular Basic with 2 vCPUs/4 GiB and the current Intel pool with 4 vCPUs/8 GiB. The
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
sum to $0.178560 at current rates. Each trial builds the
same exact baseline and candidate, runs only the four paired qualification
rounds, records CPU identity, build time, relative-ratio stability, billable lifetime,
conservative charge upper bound, and projected full-acceptance charge, then
confirms destruction. A failed qualification is reported as ineligible, and
the matrix fails if neither profile qualifies. Diagnostic mode is validated
independently on the guest and coordinator: it cannot run the
eight-leg comparison, append history, or promote evidence into the repository.
The 2-GiB and extra-disk shapes remain excluded because their sub-cent savings
do not repay the added OOM risk or hourly rate during the protocol's fixed cooldown windows.
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

Qualification sidecars never enter the accepted A/B analysis. DigitalOcean v3
admits four counterbalanced baseline/candidate pairs by the spread of their
relative ratios, not by absolute speed across minute-separated samples. Each
pair gets one committed idle window and cooldown; its second leg follows
immediately, and both timed legs retain the contaminant and steal checks. The
eight accepted legs use the same four-pair treatment. Any declared
contaminating-process or out-of-budget steal violation discards the whole block. Only
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

The physical fallback automation remains available. Vultr uses an actual
`vbm-4c-32gb` bare-metal server in `sgp`,
running Ubuntu 24.04 x64. Vultr rounds usage up to a one-hour `$0.164` minimum.
Because physical provisioning can remain `pending` well beyond fifteen
minutes, the committed 110-minute lifecycle permits a conservative one-hour
activation wait and has a two-hour `$0.328` compute ceiling; deletion still
starts with an eight-minute reserve and normally keeps a prompt deployment
inside its first billing hour.

Create a service user attached to a role with exactly the Vultr-managed
policies **Deploy New Servers**, **Manage Servers**, and **View Servers**.
Allowlist the coordinator host's current public IPv4 address as a `/32` in
Vultr's API access control. Enter its API key only through the private TTY
prompt; the coordinator stores it mode `0600` under the ignored
`.rustia-benchmark-state/vultr/` directory:

```bash
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-vultr-auth'
ssh ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-vultr-plan'
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-vultr'
```

The Vultr coordinator uses a unique ephemeral SSH key only in server user data,
checks the exact region, plan, Ubuntu image, source identities, clean checkout,
list/create/delete authority, and rejects any activated host where the runtime
virtualization gate does not report real bare metal. It retains the same frozen
qualification, fixed work, repetitions, eight-leg order, cooldowns, bootstrap,
zero-steal rule, verdict, and conditional three-row publication protocol. The
unique label is journaled before creation, ambiguous creates are reconciled by
that label, and API list results must confirm deletion before evidence or
history can be promoted.

Vultr crash recovery is likewise provider-isolated:

```bash
just bench-vultr-status
just bench-vultr-unlock
just bench-vultr-destroy
just bench-vultr-destroy RUN_ID
```

Two Vultr bare-metal orders remained pending through their committed activation
windows and were deleted without running the benchmark. The next physical-host
fallback is Scaleway Elastic Metal `EM-A610R-NVMe`: one AMD Ryzen PRO 3600 with
6 cores/12 threads, 32 GiB ECC DDR4, and two 1 TB NVMe devices. The coordinator
selects only the frozen Ubuntu 24.04 LTS image and an `available` or `low` stock
entry in the committed zone order. Its hourly-price ceiling is EUR 0.110 and
the 110-minute lifecycle has a conservative EUR 0.220 compute ceiling before
tax while the coordinator and provider API remain reachable, with the final
eight minutes reserved for cleanup. This is not a provider-side hard cap: a
coordinator, network, or API outage can extend billing until deletion succeeds.

Create a Scaleway application scoped to the benchmark project with only
`ElasticMetalFullAccess` and `SSHKeysFullAccess`, then create an API key for that
application. Enter the project UUID and secret-key half only through the private
TTY prompt; they are stored mode `0600` under the ignored
`.rustia-benchmark-state/scaleway/` directory:

```bash
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-scaleway-auth'
ssh ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-scaleway-plan'
ssh -t ${RUSTIA_COORDINATOR_SSH} 'cd /workspace/src/rustia && just bench-scaleway'
```

The Scaleway coordinator creates a unique per-run IAM SSH key and embeds a
separate pinned host key through cloud-init. It journals both resources before
creation, reconciles ambiguous responses by exact ownership fields, verifies
the activated machine is physical and matches the frozen CPU topology, and
confirms server deletion before deleting the IAM key. Neither accepted evidence
nor history can be promoted until both resources are confirmed absent.

Scaleway crash recovery remains provider-isolated:

```bash
just bench-scaleway-status
just bench-scaleway-unlock
just bench-scaleway-destroy
just bench-scaleway-destroy RUN_ID
```

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

Freeze only from a clean committed checkout, commit the generated config, plan
without billing, then run the complete reconstruction on one ephemeral Linode:

```sh
just bench-linode-backfill-freeze aee7749 bench/cloud/linode-history-backfill.json
just bench-linode-backfill-plan bench/cloud/linode-history-backfill.json
just bench-linode-backfill bench/cloud/linode-history-backfill.json
```

The dedicated shape is capped at 55 minutes and one rounded Linode billing
hour. Historical source commit time is retained separately from honest
measurement time; renderers order the reconstructed trend by the frozen target
list rather than pretending the measurements happened in the past.

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
