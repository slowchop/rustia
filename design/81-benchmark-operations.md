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
that one attesting entrypoint; the four on-host runners may not be invoked from
a workflow at all.

The qualification record carries the machine's measured repeatability. Quote it
per workload: spread scales inversely with per-unit cost, so the same host reads
0.32% worst on `dense_tables` and 5.74% on `closures_upvalues`.

### Known gap: the A/B acceptance runner

`tools/run-linux-fixed-benchmark.js` still assumes the rented model — root,
`/root/.cargo`, and permanent masking of the host's package timers. Nothing
executes it any more: no workflow reaches it, and the only remaining caller is
`validate`, which reads a frozen config and takes no timing. It therefore has not
been ported, and porting it is the prerequisite for running a full A/B acceptance
campaign on the owned host rather than a screening comparison. The three runners
that do execute there — history, matrix, memory — have had those assumptions
removed and are checked for their absence.

### Cost

No timer may start a billed instance. The retired cloud matrix stays reachable
by explicit dispatch only, because it remains the sole producer of the
cross-engine reference every published Rustia number is normalised against;
running it is a deliberate act that costs money. When it moves to the owned
host, the cloud lifecycle and its cost ceilings, ownership journal, and deletion
gate retire with it.

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

## Automation

1. Shared self-hosted CI performs correctness and artifact validation only.
2. A free relevance check identifies pending Rustia performance commits.
3. A weekly dependency check proposes stable engine updates as ordinary review
   changes; merging an update marks the landscape pending but spends no money.
4. An operator explicitly plans and starts a qualified progress, bridge,
   landscape, or candidate campaign.
5. Promotion requires a complete cohort, semantic checks, provenance, accepted
   host gates, and confirmed resource deletion.
6. Artifact-only publication validates committed evidence, renders the report,
   and pushes the data/Pages repositories. Publication never invokes a timing
   harness or cloud-provider create endpoint.

## Statistics and presentation

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

- No scheduled job may create a billed host.
- Progress runs are batched around relevant changes and decisions.
- Landscape runs occur at most monthly by default.
- Candidate runs require a concrete, credible optimization hypothesis.
- Every campaign freezes a maximum runtime, rounded billing ceiling, cleanup
  reserve, and exact resource scope before the first billable request.
