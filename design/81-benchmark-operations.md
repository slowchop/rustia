# Benchmark operations and public freshness

> **Status: [ROLLING OUT] (2026-08-14).** This document defines how Rustia
> performance evidence is measured, refreshed, and published. It complements
> [`80-bench-suite.md`](80-bench-suite.md), which defines the harness and
> measurement protocol.

## Goals

The benchmark product must answer two different questions without blending
their evidence:

1. **Rustia progress:** did Rustia become faster or slower under a stable
   protocol and fixed reference cohort?
2. **Current engine landscape:** how does current Rustia compare with the
   latest stable engine releases supported by the harness?

It must also preserve candidate A/B experiments as research evidence, expose
when results are stale or paused, and never start billed infrastructure merely
because a scheduled CI job fired.

## Runner classes

The boundary is a property of the machine, not of a workflow filename. Two
classes exist, declared in `bench/operations-policy.json` and enforced by
`tools/check-workflows.js` and `tools/render-benchmark-operations-status.js`.

### `general-linux` — shared, never a timing authority

The general Actions pool is self-hosted, shared, and normally CPU-busy. It is
valid for deterministic work whose result does not depend on wall-clock speed:

- formatting, builds, Clippy, unit/integration and conformance tests;
- adapter compilation and checksum/correctness gates;
- benchmark-tool self-tests and committed-artifact validation;
- performance-relevance and dependency-update detection;
- rendering and publishing already accepted benchmark artifacts.

**These are not benchmark hosts.** No result measured on them may enter a
performance claim, regression trend, or current-engine ranking. Wall-clock and
resource-sensitive tests must use deterministic clocks, generous timeouts, or a
qualified host. The policy makes this mechanical: a class that is not a timing
authority may hold no role matching `measurement-`, and a workflow bound to such
a class may not name any measurement command.

### `bench-solo` — one owned, qualified machine

Timing happens on a single owned host, quiet by construction: cores held off the
scheduler, SMT disabled, timed work pinned to one isolated core, and one job on
the box at a time. It is a timing authority precisely because those properties
are checked rather than asserted — `tools/run-dedicated-benchmark.js` compares
the machine's own report against the frozen config before it ships anything,
including that every pinned core appears in the kernel's isolated set and is
held at its ceiling. A pinned core the scheduler still uses is the failure this
arrangement exists to prevent, and it does not show up in the timings.

Neither does a pinned core left on a dynamic frequency governor, and that one
was measured before it was checked. On an isolated, tickless core nothing
drives the governor's next decision while a single process runs, so the first
process after a long idle can run to completion at the floor: the first leg of
an acceptance run read `dense_tables` at a median of 65,325 ns/unit against
37,824-39,205 on every other leg of its side (a ratio of 1.68 to the core's
3.8/2.2 GHz range of 1.73), the next process on the same core ran at full
speed, and the leg passed every idle and steal gate. The attestation therefore refuses a pinned core
unless it runs the `performance` governor or has `scaling_min_freq` equal to
`scaling_max_freq`, and refuses it again when the kernel exposes a boost knob
for it that is on: the governor holds the ceiling, boost lets the part float
above it on thermal headroom. The acceptance summary names any leg that sits
apart from the rest of its side, so the next such event is visible from the
workflow rather than from a login.

At most one class may be authoritative, so the shared pool cannot be promoted
later by adding a second. Measurement is reachable from a workflow only through
that one attesting entrypoint; the four on-host runners may not be invoked from
a workflow at all.

The qualification record carries the machine's measured repeatability. Quote it
per workload: spread scales inversely with per-unit cost, so the same host reads
0.32% worst on `dense_tables` and 5.74% on `closures_upvalues`.

### Waiting for the host

Exclusivity and order are separate mechanisms. `bench-solo` holds exactly one
runner, so a second job sent to it waits in GitHub's runner queue rather than
sharing the machine. That alone keeps a pull-request screening run off the
pinned cores while a measurement holds them, and it stops being true the moment
a second runner joins the group.

The four lanes that hold the host for a full measurement — the per-commit
series, the cross-engine matrix, the memory cohort and A/B acceptance — also
share the concurrency group `rustia-bench-host-serial`, declared with
`queue: max` so that every waiting run is kept. The key is load-bearing. A
concurrency group otherwise keeps one pending run, and a newly queued run
cancels it; `cancel-in-progress: false` protects only the run already in
progress. On 2026-09-15 that cancelled the per-commit run for `f49646f` (Actions
run 34921991922) two seconds after a matrix dispatch joined the group, while the
run for `08cf383` was measuring. #39 changed no engine code, so no row was lost;
an engine-changing commit would have lost its row with nothing reporting it.
With `queue: max` GitHub keeps up to 100 pending runs and starts them first in,
first out by when each began waiting, and cancels any beyond the hundredth. Its
documentation adds that the order is not guaranteed, because start times vary.

The group stays, rather than leaving all waiting to the runner queue, because
the per-commit series depends on order and the runner queue documents none.
Rows are appended in the order runs execute, the published chart draws them in
file order, and the unchanged-binary gate compares a new build with the last
row. If a per-commit run does execute after a later commit's run has recorded
a row, `check-benchmark-series.js` refuses the append before anything is
pushed — the recorded commit is not an ancestor of the one being measured — so
an inversion costs that row visibly rather than bending the line. The exception
is a build identical to the last row's binary: it records nothing, and the
change is attributed to the later commit. The matrix, memory and acceptance
lanes measure main's tip or commits named at dispatch, and do not depend on
order.

Screening stays outside the group: it cancels its own superseded runs per pull
request, and GitHub rejects `cancel-in-progress: true` combined with
`queue: max`. The publisher stays outside too, because the memory lane calls it
while holding this group. `tools/check-workflows.js` pins the exact block in
the four lanes, allows no second concurrency block beside it, and rejects the
group name in every other workflow.

**No catch-up.** A per-commit run that fails, is cancelled by hand, or waits
more than GitHub's 24-hour limit for a self-hosted runner records no row for
its commit, and nothing measures that commit later. Re-running it recovers the
row only while no later commit has recorded one; after that the ancestry rule
refuses the append. The series already has three such gaps: `0fce032` (#14),
`50a5bfb` (#15) and `7b8d334` (#17) changed engine code, and their runs failed
on 2026-09-01 and 2026-09-02. Catch-up — each run measuring every unrecorded
first-parent commit since the last row, oldest first — would close those gaps
and make the lane independent of run order. It is not built: it needs the
`main` command to measure a commit other than HEAD, a per-run budget inside the
30-minute timeout, and a decision on whether a point measured long after its
commit landed is drawn like one measured on landing. Queueing removes the loss
that prompted it; the failure gaps remain open.

### The memory cohort and its delegated cgroup

Per-VM memory is measured inside a dedicated cgroup, so the memory lane needs
something the timing lanes do not: a cgroup-v2 subtree the unprivileged CI user
can create children in. The owned host has one, and the lane completed there on
2026-09-05, producing its first cohort — `rustia-ci-070c387b173c` at commit
`070c387`, 11 Rustia rows from 99 raw samples.

The lane measures one of two cohorts, chosen by the workflow's `cohort` input.
The routine cohort keeps the bare `memory --local` line, because the measured
commit's own tools run and commits from before the flag existed must stay
measurable; only the cross-engine cohort is passed through as `--cohort`, and
only commits that carry its freezer can measure it. Both go through the same attesting entrypoint
and inherit one frozen protocol — samples, bootstrap, thresholds, host block,
pinned cores — and differ in the engine set alone. `rustia-only`, the default,
is the routine per-commit cohort: 11 rows from 99 samples, and the only one
that runs the wasm32 quota probe. `cross-engine` measures the thirteen matrix
engines under the protocol the rented lane measured as `pre-reclamation-18b58d8`:
110 rows from 990 fresh processes. It is the cohort that puts the competitor
rows and the mlua/Luau/JIT memory reference on the same machine as the Rustia
rows they are compared with; one such cohort returns the comparison, and it is
re-run when a competitor dependency moves rather than on a cadence. Its build
carries twelve competitor crates the rustia-only build does not, so its first
run on a cold cache is the long one; the workflow's 75-minute timeout is the
lane's only runtime ceiling, since the host block's `max_runtime_minutes` is
inherited from the shared config and not enforced by the memory runner. In the
published data each engine's rows come from the newest cohort that measured it,
so a rustia-only cohort after a cross-engine one swaps the Rustia rows and
keeps the competitors. No cross-engine cohort has been measured on the owned
host yet; the comparison stays pending until the first is dispatched.

Root is one way to have such a subtree. The other, which keeps the runner
unprivileged and is the arrangement in place, is a systemd-delegated subtree
that *contains the job's own process*: on the unit that runs the runner,
`Delegate=memory` plus `DelegateSubgroup=` (so the unit's own cgroup holds no
processes and may enable the controller for its children) and no
`ProtectControlGroups=`. A subtree delegated beside that
unit — a lingering `user@<uid>.service` — is no use, because cgroup v2 refuses
to move a process across a common ancestor the mover cannot write. The tools
find the delegated root by walking up from the process's own cgroup
(`bench/cgroup-delegation.js`) rather than from a path guessed from the uid,
check it before anything is cloned, and name which of the four faults — nothing
delegated, read-only mount, delegated root still holding processes, no memory
controller — is the one present.

**The check must not be relaxed to make the lane pass.** Without the cgroup there
is no per-VM figure — the accounting is the cgroup's — so a run that skipped it
would produce numbers that look like measurements and are not. The lane is
dispatch-only and the check is a preflight that fails closed before anything is
cloned, so a host that loses the delegation costs a dispatch and nothing else;
the alternative, restoring the cloud lane, would reintroduce the ability to
spend money to work around a one-line host setting.

### A/B acceptance on the owned host

`.github/workflows/acceptance-bench.yml` answers what screening cannot: not "did
this pull request move anything" against a merge base, but "did this change move
this workload, and by how much" for a pair of commits named explicitly. It takes
a baseline, a candidate, and an even number of legs per side between 2 and 16,
and measures on `bench-solo` through the same attesting entrypoint as every
other lane. It is dispatch-only — a run holds the host for the length of its
passes, so it is never on a timer and never on a push — and it records nothing:
the reading lands in the job summary, not in the published history, because a
comparison of two arbitrary historical commits is evidence for a decision
rather than a row in a series.

It measures the pair in eight passes by default (`--passes` takes any even count
from two to sixteen), A B B A repeated, at the screening calibration's
repetition count, and reads each workload through the same hierarchical
bootstrap the screening lane uses, against that calibration's thresholds. The
summary reports, per workload, the delta, the interval, the threshold and the
direction, and — because one leg can widen an interval across zero on its own —
names any leg whose median sits more than the larger of 5% and three times the
workload's threshold from the median leg of its side, by the pass it came from.
A side of fewer than three legs has no median to stand apart from and is never
flagged.

The frozen owned-host config, `bench/cloud/dedicated-core2-tape-sparse.json`,
is the one place the repository records what the host guarantees. The matrix
and memory config generators copy its provider block, and the history freezer
cross-checks host identity against it, so `tools/check-dedicated-config.js`
validates it — exact provider keys, SMT off, the timed core inside the pinned
set — on every `just check-benchmark-results` and release-preflight run. The
rented-model A/B runner that once carried that check is deleted; the three
runners that execute on the host — history, matrix, memory — are checked for
the absence of its assumptions.

### Cost

**Nothing in this repository creates or pays for a cloud instance.** Not on a
timer, not on a push, not by dispatch, and not by a justfile recipe. Every lane
— the reconstruction, the per-commit series, pull-request screening, A/B
acceptance, the cross-engine matrix and the memory cohort — runs on the owned
host, so the cloud lifecycle's cost ceilings, ownership journal and deletion
gate have nothing left to govern and are retired with it.

One file remains from that model. `tools/run-linode-benchmark.js` is the only
home of the matrix and memory acceptance rules, and the owned-host coordinator
borrows them rather than carrying a second copy that agrees until one is edited.
It is documented as retired at the bottom of the file; nothing runs its
lifecycle, release preflight runs only its offline self-test, and it goes when
those rules move to a shared module.

## Evidence lanes

### 1. Rustia progress

- Measure Rustia after performance-relevant compiler, parser, VM, toolchain, or
  dependency changes accumulate.
- Use the existing fixed five-workload Linux protocol.
- Keep comparison-engine versions and the reference cohort pinned within an
  era.
- Measure the newest pending commit first. Backfill intermediate commits only
  when needed to locate a regression.
- Start a new era when the harness, workloads, toolchain, qualified host shape,
  or protocol changes materially.

### 2. Current engine landscape

- Run monthly by default, and before releases or major performance claims.
- Compare current Rustia with the latest stable engine releases supported by
  the adapters. Upstream development branches and nightly builds are separate
  experimental evidence, never the default landscape.
- Record the exact engine version/source commit, adapter version, execution
  mode, Rust toolchain, Rustia commit, host identity, protocol hashes, and
  measurement window.
- Require equivalent checksums and supported semantics before accepting a row;
  render unsupported workloads as `not comparable`.
- Store immutable dated snapshots under `bench/landscape/` and point
  `bench/landscape/current.json` at the accepted current campaign.

### 3. Version bridge

When a primary reference engine changes, measure its old and new versions in
the same qualified cohort. The bridge attributes discontinuity to the engine
upgrade and prevents a silently moving reference from rewriting Rustia's
historical trend.

Luau interpreter and verified Luau JIT are the primary reference. Other
languages and engines remain useful secondary landscape comparisons, but do
not substitute for Luau-semantic parity.

### 4. Candidate experiments

Keep controlled ABBA/BAAB evidence separate from progress and landscape data.
Retain accepted, rejected, and inconclusive experiments with raw samples,
baseline/candidate identities, host gates, uncertainty, and the declared
decision threshold. A rejected candidate never enters the progress trend.

### 5. Pull-request screening

A reading for the author of a change, produced on the same qualified host and
retained nowhere. `.github/workflows/pr-bench.yml` measures the branch against
the merge base of the branch and its target — not the target's tip, which would
attribute an unrelated commit landing mid-review to the pull request — across two
passes with the order reversed in the second, so the sides interleave A B B A.
The base is taken from the pull request event as a commit id: the job drops its
credentials at checkout and so cannot reach the remote, which is a property worth
keeping rather than working around.

Screening is a distinct lane from candidate experiments and must not be confused
with one. It runs a short protocol, publishes nothing, and holds no write access
to either repository. A candidate experiment is what settles whether an
optimisation is adopted.

The same qualified PR job runs the candidate-only live-value serde
microbenchmark through `run-dedicated-benchmark.js live-serde`. It pins the
benchmark binary to the attested core and compares direct live deserialization
with the explicit `OwnedValue` path using the example's A B B A order. The
summary is review evidence only: it asserts equal decoded values, writes no
history row, and does not revive the historical 30% claim.

**Thresholds are measured on the host rather than declared.**
`bench/pr-screening.json` is produced by an A/A run: one revision measured four
times, passes 1 and 4 compared against 2 and 3 through the ordinary analyzer
with `--allow-identical-commits`. Both sides are the same binary, so the
interval is the host's own spread, and each workload's threshold is its widest
end rounded up. There is no default threshold, because a declared figure that
sits inside the host's spread reports movement on changes that did nothing.

The A/A run is also the protocol's canary, and it has already earned its keep.
Four passes admit three balanced splits, all of which are analysed. On the first
real run `sparse_tables` read as 1.2% faster than itself under `1,3 vs 2,4`,
with an interval excluding zero — monotonic session drift, not a code fault,
under an arrangement where each candidate pass immediately follows its baseline
so the drift does not cancel. The screening lane orders its passes A B then B A,
which straddles the run, so the canary fails only on that straddling split: it
is the arrangement whose indistinguishability the lane actually claims. The
other splits still widen the threshold and are recorded per workload, because
whatever they show is movement this host can produce.

That result is what makes the A B B A ordering a measured requirement rather
than a stated precaution. Recalibrate whenever the machine changes; adopting a
calibration is a deliberate commit.

Because the lane reads five workloads at once, it screens at a 99% interval
rather than 95%: five independent 95% readings give roughly a 23% chance one
moves on a null change, and more repetitions narrow the intervals without
changing that rate.

**Untrusted pull requests.** The lane executes the branch's code on the machine
that produces every published number, which holds persistent cargo roots and
target directories later runs read. `pr-bench.yml` therefore carries
`if: github.event.pull_request.head.repo.full_name == github.repository`, pinned
by `checkPrBench` so it cannot be dropped silently, and fork pull requests get
correctness CI only. This costs nothing while the repository is private, where
every author is already trusted; it is in place now because the day it starts to
matter is the day the repository goes public. Going public should additionally
split the runner identity — a trusted lane for `main` and the weekly matrix, and
an `--ephemeral` unprivileged lane for pull requests — and require approval for
outside collaborators.

## Freshness model

The generated machine-readable status is `bench/operations-status.json`. Its
policy source is `bench/operations-policy.json`.

- **current:** the newest performance-relevant commit is represented by the
  accepted progress cohort.
- **pending:** relevant commits exist after the accepted cohort.
- **stale:** a time-bounded lane exceeded its refresh interval.
- **paused:** the lane or its publication automation is intentionally not
  operating.

The public report must show measurement time, measured commit, newest known
relevant commit, pending count, and the next operator action. It must never
look continuously updated when measurement or publication is paused.

Pull requests use merge commits rather than squash or rebase. Freshness is
derived from reviewed commit identities, so rewriting those identities while
landing a change makes the generated operations view stale immediately after
an otherwise green merge.

The weekly matrix publication depends on that rule. GitHub holds pull-request
runs on a pull request the workflow token opens until a person approves them,
and the landing helper merges within seconds, so the publication commit carries
GitHub's CI-skip marker and its pull request starts no checks. A squash or
rebase would copy the marker onto `main`; GitHub still offers both there, so
the rule is a convention rather than a setting. The same landing means `main`
cannot require status checks: a skipped check stays expected and the merge is
refused. The merge starts no push workflows, so the workflow dispatches CI on
`main` after landing.

## Automation

1. Shared self-hosted CI performs correctness and artifact validation only.
2. A free relevance check identifies pending Rustia performance commits.
3. A weekly dependency check proposes stable engine updates as ordinary review
   changes; merging an update marks the landscape pending but spends no money.
4. An operator explicitly plans and starts a qualified progress, bridge,
   landscape, or candidate campaign.
5. Promotion requires a complete cohort, semantic checks, provenance, and the
   accepted host gates — the attestation against the frozen config plus the
   committed checkers.
6. Artifact-only publication validates committed evidence, renders the report,
   and pushes Pages directly. Data-repository updates are one commit on a
   fresh clone of that repository's `main`, pushed there as a fast-forward by
   the deploy key through the ruleset's deploy-key bypass, with a bounded
   fetch-rebase-retry when another publication landed first. Publication
   never invokes a timing harness, a cloud-provider create endpoint, or a
   GitHub-hosted runner.

## Statistics and presentation

### Grabby build results and one dashboard publisher (2026-09-09)

Gak confirmed `https://slowchop.github.io/rustia-bench-data/` as the destination for
Grabby's reviewed build-time results. The separate `#grabby-builds` section uses
`bench/grabby-builds/results.json`, never Rustia workload rows, rankings, histories,
memory cohorts or claim gates. It presents the two sequential ci-bench A/B runs
and their exact source-report identities, preserving both contemporary LTO-off
controls, cold n=1, four paired edits and runtime/debugging costs.

Only selected public benchmark fields are copied into this repository and the
public `grabby-builds.json`. Full internal reports, machine load/swap diagnostics,
chat quotes and raw logs stay in their original repository. SHA-256 values identify
the immutable original reports, not the selected JSON's bytes. The selection was
compared read-only against those reports; its validator checks field allowlists,
experiment identity and paired arithmetic. It does not repeat the earlier raw
compiler, snapshot or screenshot audits or independently approve new adoption.

`publish-benchmark-data.yml` is the sole Pages writer. It is also reusable by the
memory workflow after a successful data-only promotion. Historical memory code
can measure and merge memory rows but cannot render or write Pages. The publisher
checks out current main, hydrates current data-owned memory and live series, keeps
the selected Grabby source, and serializes its writes under
`rustia-benchmark-data-publication`. It never measures or needs an actions-write
token. The benchmark-host group is distinct, preventing a self-deadlock. Unlike
that group it keeps GitHub's default single pending slot on purpose: each run
renders current main and the data repository's current rows, so a newer
publication supersedes a waiting one rather than losing anything.

This preserves accepted memory/series data and keeps later memory updates from
replacing the dashboard with a historical renderer. Publication uses the data
snapshot it reads, not a transaction spanning concurrent measurement appends.
Failure can be recovered by dispatching only the publisher; no measurement rerun
is needed. The local page and selected-data validation are implemented; final PR
CI, merge, publication and live verification remain delivery gates.

- Keep cold preparation, first call, warmed execution, throughput, and memory
  as separate dimensions.
- Retain ordered raw samples and publish sample count, median, variability or
  interval, and the protocol noise floor alongside min-time compatibility
  fields.
- Classify changes as improved, regressed, or indistinguishable under a
  prospectively declared threshold.
- Maintain representative application workloads alongside stable microbench
  families; do not roll them into one opaque overall score.

## Rollout

1. **Complete:** add generated freshness metadata and a prominent public
   paused/pending status.
2. **Complete:** restore artifact-only automatic publication.
3. Separate progress history from immutable landscape snapshots.
4. Add complete engine-version provenance and version-bridge validation.
5. Add weekly stable-engine update proposals.
6. Run and publish the first latest-stable landscape campaign.
7. Add relevance detection to CI without granting it measurement or billing
   authority.

## Cost policy

- No job can create a billed host, because no path in this repository can.
- Progress runs are batched around relevant changes and decisions.
- Landscape runs occur at most monthly by default.
- Candidate runs require a concrete, credible optimization hypothesis.
