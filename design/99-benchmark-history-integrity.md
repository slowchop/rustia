# Benchmark graph publication and history integrity

Date: 2026-10-27 (Australia/Hobart).
Status: local fixes validated; fresh A/A calibration failed; existing-graph deployment authorized and pending required checks.
Baseline: `a62ef708e63c02fdea5630c32e3f8c78d173b82a`.

## Failure and evidence

The per-commit measurement and artifact publisher both started on the merge push.
The [publisher](https://github.com/slowchop/rustia/actions/runs/40423316604)
finished before the
[measurement](https://github.com/slowchop/rustia/actions/runs/40423316647)
saved five new rows. The graph build used data from before those rows existed.
The data repository subsequently contained 70 live rows across 14 commits, ending
at the baseline above, while the graph stopped at `90bc2ae4`. Both workflows
reported success. A successful measurement was not proof of a current graph.

The graph also ignored the host-control ledger. It joined measurements across
flagged differences and claimed that one physical machine made all differences
engine changes. A single machine can change its kernel, packages and conditions;
the recorded controls directly contradict that stronger claim.

## Retained host-control investigation

The inspected ledger contains 13 controls. All compare byte-identical binaries;
seven exceed one or more calibrated screening limits. Each control compares
against the source revision's original row, not against the preceding control.
Multiple findings against one revision are not independent slowdowns and must
not be added together.

| Control date (Hobart) | Source revision | Outside screening limits |
| --- | --- | --- |
| 2026-10-02 | `911bad60` | Sparse tables +3.543% |
| 2026-10-10 | `911bad60` | Dense tables +3.297% |
| 2026-10-14 | `911bad60` | Dense tables +2.596% |
| 2026-10-15 | `911bad60` | Dense tables +2.717% |
| 2026-10-16 | `911bad60` | Closures -4.513%; iteration +8.880% |
| 2026-10-22 | `b22feb2b` | Math/table +3.766% |
| 2026-10-24 | `9b12a5a8` | Dense tables +9.233% |

These are execution-time differences in the retained controls, not accepted
source-change regressions. Positive values mean more time. The 24 October
[failed run](https://github.com/slowchop/rustia/actions/runs/40018239279)
retained its finding as designed. Its package-set fingerprint changed while
the kernel, microcode and binary matched the preceding control's recorded
metadata. This narrows the observed change to software/measurement conditions;
it does not identify a specific package or prove a persistent 9.233% host cost.
The package inventory is hashed rather than exported, so the ledger alone cannot
name the package difference. Short control runs also retain measurement noise.

A later successful host check means the fingerprint has not changed since the
last recorded observation. It does not review or clear earlier findings. The
paired PR #74 acceptance remains separate evidence; historical graph points are
not a substitute for its within-session comparisons.

## Implemented local correction

1. The series data step sets a job output only after its fast-forward data push
   succeeds. A dependent call to the existing reusable publisher requires that
   output and uses `always()`. The drift failure is a separate step, so the
   successful data-step output remains available even when drift fails the job.
   A failed push does not authorize graph publication. Existing artifact-only
   triggers may still run early, but the dependent publication reads the newly
   saved data afterward. It uses the same sole Pages writer and concurrency group.
2. The publisher hydrates the host-control ledger alongside live rows. The series
   validator and renderer reject fingerprinted history without its ledger.
   Ledger validation checks identities, measurement order, all five workloads,
   positive values and verdict arithmetic; inconsistent stored flags fail closed.
3. The graph displays the complete control ledger, the latest per-commit
   measurement and separate host periods at flagged controls. Its table computes
   change within a displayed period only. Reconstructed source dates still set
   the horizontal position, while actual measurement times determine the host
   period. No measured value, timestamp, binary hash or checksum is corrected,
   removed or overwritten.
4. The graph describes observed ratios to a separately measured current
   competitor reference. It no longer says that every historical movement is an
   engine change. Missing early software fingerprints and short-control limits
   are explicit. The daily status retains the count of earlier flagged controls.

With the inspected live data, the preview retains all 89 complete historical
scores in four nonempty host periods (82, 4, 1 and 2 points). The later period
numbers can skip because some flagged controls had no new source measurement
between them. This is an honest gap, not missing results.

## Fresh diagnostic and delivery

The existing private screening-calibration workflow was dispatched on `main` to
compare one revision with itself on `ci-bench`. It records no series rows, changes
no thresholds and publishes no graph. Its completed result must be inspected
before making any new statement about current repeatability. No benchmark is
run locally. Do not replace the historical ledger with a fresh calibration or
treat today's repeatability as proof that an older host transition was harmless.

The completed [fresh calibration](https://github.com/slowchop/rustia/actions/runs/40462439654)
failed its balanced A/A canary on sparse tables: +0.920%, with a 99% interval of
+0.30% to +1.29% for split `1,4 vs 2,3`. All four passes produced the same binary.
The non-straddling math/table split `1,2 vs 3,4` also excluded zero, at +0.882%
with an interval of +0.16% to +1.64%; that is recorded session drift, not the
canary failure condition. The workflow refused to write new thresholds. No
threshold was adopted and no retry was made to obtain a passing result.

The [dated review record](99-benchmark-host-calibration.json) drives an explicit
failed-calibration notice in the graph. It is reviewed evidence as of the stated
date, not an automatic live health feed. Historical observations remain
provisional. The graph fix makes that limitation visible rather than claiming
that the runner is fixed.

The raw-sample inventory on the runner was requested read-only through the
existing configured SSH connection. Filesystem permission was denied under that
account. No alternate account, privilege escalation or permission change was
attempted. The private failed-run logs and per-pass summaries were retained;
identifying a physical cause requires authorized access to the raw pass data or
a separately specified evidence-retaining diagnostic. The available evidence
does not justify naming a package, applying correction factors, or declaring
current repeatability healthy.

Re-measuring historical revisions under one frozen current environment would
provide a separate coherent cohort, but must retain the old records and its own
provenance. It is not necessary to invent correction factors or silently rewrite
the existing measurements to present them honestly now.

Publication sequencing, failure gating, ledger validation, graph segmentation,
reconstruction measurement-time handling and warning text have focused regression
checks. Full validation, private delivery and inspection of the actually rendered
graph remain required before deployment is accepted. Full release preflight
completed with 98 Rust test summaries, 1,466 passes and zero failures before the
final dated diagnostic annotation. The affected workflow/renderer/ledger checks,
package checks and quick inventory were rerun for that annotation. The repository remains
private, packages remain unpublished, and the existing public graph has not been
updated by this local preparation.

The user subsequently authorized deployment to the existing benchmark graph,
while keeping Rustia private and packages unpublished. Delivery must use the
required private PR, CI and merge gates, then verify the graph artifact and
actual published page. Host-drift investigation continues separately; graph
deployment does not mean the failed calibration has been resolved.
