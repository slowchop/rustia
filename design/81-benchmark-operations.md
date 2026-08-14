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

## Runner boundary

GitHub Actions runners are self-hosted, shared, and normally CPU-busy. They are
valid for deterministic work whose result does not depend on wall-clock speed:

- formatting, builds, Clippy, unit/integration and conformance tests;
- adapter compilation and checksum/correctness gates;
- benchmark-tool self-tests and committed-artifact validation;
- performance-relevance and dependency-update detection;
- rendering and publishing already accepted benchmark artifacts.

They are not benchmark hosts. No result measured on those runners may enter a
performance claim, regression trend, or current-engine ranking. Wall-clock and
resource-sensitive tests must use deterministic clocks, generous timeouts, or
an isolated qualified host.

Fresh timing remains an explicit operator-started lifecycle on an ephemeral,
qualified benchmark host. The coordinator must retain its existing hard cost
and runtime ceilings, resource ownership journal, and confirmed deletion gate.

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
