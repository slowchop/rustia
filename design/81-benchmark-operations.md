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
including that every pinned core appears in the kernel's isolated set. A pinned
core the scheduler still uses is the failure this arrangement exists to prevent,
and it does not show up in the timings.

At most one class may be authoritative, so the shared pool cannot be promoted
later by adding a second. Measurement is reachable from a workflow only through
that one attesting entrypoint; the on-host runners may not be invoked from a
workflow at all.

The qualification record carries the machine's measured repeatability. Quote it
per workload: spread scales inversely with per-unit cost, so the same host reads
0.32% worst on `dense_tables` and 5.74% on `closures_upvalues`.

### The memory cohort and its delegated cgroup

Per-VM memory is measured inside a dedicated cgroup, so the memory lane needs
something the timing lanes do not: a cgroup-v2 subtree the unprivileged CI user
can create children in. The owned host has one, and the lane completed there on
2026-09-05, producing its first cohort — `rustia-ci-070c387b173c` at commit
`070c387`, 11 Rustia rows from 99 raw samples.

The lane measures one of two cohorts, chosen by the workflow's `cohort` input.
The measured commit's own tools run the measurement, so the routine cohort
keeps the bare `memory --local` line for commits that predate the `--cohort`
flag; only the cross-engine cohort passes it, and only commits that carry its
freezer can measure it. A commit from before host-labelled memory rows cannot
be measured at all: one whose `bench/analyze-memory-samples.js` does not carry
the host-label literal as a whole line is refused right after the checkout,
before the build, because its rows could not name their machine by label and
the data repository takes no row that cannot. `tools/check-workflows.js` pins
that gate and requires the analyzer to keep the literal as a whole line, since
an indented or annotated copy would refuse every commit. The merge and the
push into that repository do not run the measured commit's tools (see "Merge."
under "Row provenance"). Both cohorts go through the same attesting entrypoint
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

The frozen owned-host config, `bench/cloud/dedicated-core2-tape-sparse.json`,
is the one place the repository records what the host guarantees. The matrix
and memory config generators copy its provider block, and the history freezer
cross-checks host identity against it, so `tools/check-dedicated-config.js`
validates it — exact provider keys, SMT off, the timed core inside the pinned
set — on every `just check-benchmark-results` and release-preflight run. The
rented-model A/B runner that once carried that check is deleted; the three
runners that execute on the host — history, matrix, memory — are checked for
the absence of its assumptions. The checker also requires `provider.host.name`
to be a label, never an address: every lane's config carries that name, and
the runners stamp it on every owned-host row as `host` (see "Row provenance").

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

## Row provenance

Every measurement row names the machine that produced it, in a `host` field
carried from the runner through the harness into `history/runs.jsonl` and on
into the published tables. The label is provenance published in row data and
nothing else: no tool resolves it, connects to it, or derives an address from
it, and the owned-host coordinator refuses, before any ssh or scp, a
destination whose host is a label this repository publishes
(`sshDestination()` in `tools/run-dedicated-benchmark.js`; `bench/README.md`
says what it compares). An owned machine's label is the name in its frozen
config; a rented instance had no stable identity to name, so its honest label
is its machine class (`linode-<region>-<size.id>`). The two are different
strings, which is the property that matters.

The rule the field enforces is that **a published table is one machine's
readings**. Both `bench/current-results.json` and
`bench/memory/current-results.json` are per-slot merges — newest row per
(target, engine, workload) across every cohort in their history — so a partial
re-measurement refreshes the slots it covered and silently leaves the rest as
they were. On 2026-09-05 a rustia-only memory run did exactly that: 11 rows
moved to the owned host, 99 competitor rows stayed on a rented instance
measured six weeks earlier, and the metadata named the owned host for all 110. Only `row.commit` revealed the split. Every figure
on that page is a ratio between two rows, so this was not a labelling gap; it
was wrong numbers that looked measured.

The field is therefore asserted at each point a row can move, and each
assertion is a refusal:

- **Emission.** The on-host runners derive the label from the frozen config and
  pass it as `--host`; the memory analyzer requires `--host` with no default,
  because a default would be a guess published as provenance.
- **Acceptance.** `tools/run-linode-benchmark.js` — the shared matrix and
  memory acceptance rules — recomputes the expected label from the config and
  rejects rows that name anything else, so validation restates the label rather
  than trusting the runner's copy of it.
- **Publication.** `bench/publish-memory-analysis.js` refuses an analysis whose
  rows disagree about their host: a cohort is one run on one machine.
- **Merge.** `tools/merge-rustia-ci-memory-data.js`, the step that produced the
  2026-09-05 mix, now refuses to publish a merged table spanning two machines
  and names the engines on each. The merge is per engine — each engine's rows
  come from the newest cohort that measured it — so it first requires the
  generated cohort to be one machine: its rows, its `metadata.host` and its
  history line must agree, and the refusal blames the side that does not.
  Every row it keeps, generated or stored, must name its machine by label. A
  stored row for an engine the cohort measured is replaced whatever machine it
  names, or whether it names one at all, so a cross-engine cohort is how a
  table from another machine is re-measured, and a stale unlabelled row cannot
  block the run that replaces it. This is the last check before the memory
  table and history reach the public data repository, and a merge tool from
  before host labels checks nothing, so the memory lane does not run the
  measured commit's copy: it runs main's, and main's push helper, copied into
  the runner's temporary directory before the checkout detaches at the
  measured commit. `tools/check-workflows.js` pins the copies, their order and
  their invocations, and requires both tools to load only `node:` builtins so
  a copy runs outside the tree.
- **Validation.** `tools/check-benchmark-results.js` requires every history row
  to name a host, requires each published table — timing and memory — to name
  exactly one, requires that one to be the host of the runner class marked
  `timing_authority` in `bench/operations-status.json`, requires each memory
  cohort to be one machine, and exercises those refusals on fixtures before
  reading the committed data — so the machine boundary above is checked in the
  data, not only in the workflow.
- **Render.** `bench/render-current-results.js` refuses to render a table whose
  rows disagree, names the machine in the memory panel from the rows rather
  than from prose, and pins everything it draws as the current machine — the
  timing table, a ready memory table, and the memory history's latest cohort —
  to the timing authority itself rather than relying on the checker to have
  done so: the publication lane installs the data repository's memory table
  and history after the checker has run and renders from those, and the page
  is where the claim becomes public.
- **Bridge.** `tools/record-host-bridge.js`, which writes down how two
  machines compared before the earlier one's rows are retired, reads each
  side's host from its rows into `from_host`/`to_host` and refuses a bridge
  whose two sides are the same machine, or neither of which names one.

That last refusal was the open choice: label the mix or refuse it. The page
refuses, because a labelled ratio between two boxes is still a wrong number,
just an annotated one. The history charts follow the same rule from the row
data. The recorded timing chart draws only rows measured on the machine the
mlua/JIT reference was measured on — the current table's machine — because its
y-axis divides the reference's nanoseconds by each row's, and that quotient
means something only when both ran on one box; rows from any other machine, or
from before hosts were recorded, are counted in runs and named as not drawn,
whether the recorded runs or a reconstruction is the chart. A reconstruction or
a live per-commit series is divided into the same reference and is refused when
it names another machine, and a history with no recorded run from the
reference machine is refused rather than drawn against it. Within that machine
the series still splits on a gap of a day or more, and the table's improvement
column is each era against its own first run. The memory history is one
machine's series — the machine of the latest cohort — plotted against a
mlua/JIT reference measured on that same machine; that latest cohort is pinned
to the timing authority by the checker and by the page, older cohorts may name
the machine they were measured on, and the section says how many cohorts from
another machine, or from before hosts were recorded, it is not drawing.

The harness reads the field as optional when parsing history, because rows
written before it existed do not carry it and that absence is readable: it means
"measured before hosts were recorded", not "measured somewhere unnamed". A row
that *does* carry the key must carry a non-empty value, or the label would be a
way to look labelled while saying nothing. For the same reason
`metadata.hosts` is written only when every row names its machine: a list
naming just the rows that know would read as a single-machine table when it is
not. The committed history carries a host on every row, and the validator keeps
it that way.

The label is the only place a machine is named, and it is kept a label
mechanically: every file that emits or validates one accepts only
`[A-Za-z0-9_-]{1,128}`, so a value with a dot or a colon (a fully qualified
name, an IPv4 or IPv6 address) is refused before it can reach a row and again
before a row can be published. Those files are the harness's `--host`
(`bench/src/main.rs`); the memory analyzer and cohort protocol
(`bench/analyze-memory-samples.js`, `bench/run-memory-cohort.js`); the three
on-host runners (`tools/run-linux-matrix-benchmark.js`,
`tools/run-linux-memory-benchmark.js`, `tools/run-linux-history-backfill.js`);
the frozen-config checker, for `provider.host.name`
(`tools/check-dedicated-config.js`); the coordinator's acceptance rules
(`tools/run-linode-benchmark.js`); the memory publisher, for every row and
`analysis.host` (`bench/publish-memory-analysis.js`); the memory merge
(`tools/merge-rustia-ci-memory-data.js`); the operations-status renderer, for
any runner class that names a machine, as the timing authority must
(`tools/render-benchmark-operations-status.js`); and the publication gate and
the page (`tools/check-benchmark-results.js`,
`bench/render-current-results.js`). The files cannot share a module (a runner
is copied to the machine one file at a time, and the memory lane runs main's
merge tool from a copy outside the tree), so the checker's self-test reads
each of them — the twelve scripts for the literal
`const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;`, the harness for the same
character set and cap — and refuses to run if any has drifted. The merge does
not re-check stored history lines: history is append-only, so refusing over one
would make it permanent, and the page refuses a history host that is not a
label when it renders.

Raw memory samples and host evidence are copied whole into public artifacts, so
neither names the machine by its node name: the memory supervisor records none,
and the host evidence the owned host writes — the memory cohort's `host.json`
(`bench/run-memory-cohort.js`) and the matrix runner's owned-lane `host.json` —
records `uname -srvm` rather than `uname -a`. The owned lane's acceptance
refuses matrix evidence that carries a node name.

The published data repository holds the memory table and history the
publication lane installs before rendering, and it is subject to the same rule:
its rows must name their host before that lane renders again. The copies
committed here are identical to its files apart from the field.

The wasm lane is outside this rule and does not need it.
`bench/wasm/current-results.json` is rewritten whole by one invocation, and its
checker requires every run to carry the file's single commit, so it cannot
accumulate a cross-cohort mix in the first place. Its existing `runtime.host`
(`linux-x64`) is a platform string, not a machine.

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
can measure memory rows but cannot merge them, render, or write Pages: the lane
merges and pushes with main's copies of those tools. The publisher
checks out current main, hydrates current data-owned memory and live series, keeps
the selected Grabby source, and serializes its writes under
`rustia-benchmark-data-publication`. It never measures or needs an actions-write
token. The benchmark-host group is distinct, preventing a self-deadlock.

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
