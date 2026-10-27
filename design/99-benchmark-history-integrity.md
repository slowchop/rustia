# Benchmark graph publication and history integrity

Date: 2026-10-27 (Australia/Hobart).
Status: graph update deployed and verified; repeatability unresolved after a larger private control.
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

## Implemented correction

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

The initial raw-sample read was denied by filesystem ownership. A subsequent
read using the runner owner's existing sudo access recovered 24 sample files
and 28 metadata files without changing permissions or runner configuration.
The combined-file duplicates match the 20 individual pass/workload records;
all 15 split/workload intervals independently reproduce with the committed
analyzer. All four binary hashes, software fingerprints and protocols match;
timed gates accepted with zero steal. Sparse-table passes 2 and 3 consistently
ran slower than passes 1 and 4. This is pass-level variation, rather than one
isolated sample. Effective frequency, interrupts, cache/memory/process placement
and sampling uncertainty remain unresolved. These findings do not justify
naming a package, applying correction factors, or declaring repeatability healthy.

Re-measuring historical revisions under one frozen current environment would
provide a separate coherent cohort, but must retain the old records and its own
provenance. It is not necessary to invent correction factors or silently rewrite
the existing measurements to present them honestly now.

Publication sequencing, failure gating, ledger validation, graph segmentation,
reconstruction measurement-time handling and warning text have focused regression
checks. Full release preflight passed on the final reviewed commit through the
pre-push hook; independent workflow and history-arithmetic reviews found no
remaining issues. [PR #77](https://github.com/slowchop/rustia/pull/77) merged through
required CI as `891c29cb498811318825ef073f082ea9e90a0167`. Its tree matches the
reviewed commit. The [publication run](https://github.com/slowchop/rustia/actions/runs/40467806897)
succeeded, Pages built the corresponding data commit `5563a654`, and the
[served history](https://slowchop.github.io/rustia-bench-data/#history) was inspected
in a browser: 89 points, four nonempty periods, 13 controls, the latest per-commit
measurement at `a62ef708`, and the dated failed-calibration warning. Source
visibility was verified private and package publication remains disabled.

After deployment verification, the existing private acceptance workflow was
dispatched for a predeclared eight-leg-per-side control:
[run 40468299067](https://github.com/slowchop/rustia/actions/runs/40468299067), comparing
`a62ef708` with graph-change commit `ccb71374`. Their runtime and harness sources
are unchanged, but new build hashes must match before interpreting it as A/A.
It retains balanced order, five repetitions per leg and 99% intervals. The
coordinator's expected identical-binary skip is not a repeatability verdict:
all raw legs must be inspected independently. This diagnostic writes no history
rows and adopts no thresholds. Host-drift investigation continues separately;
graph deployment does not mean the failed calibration has been resolved.

### Larger control result

Recovered 192 evidence files, verifying their remote byte hashes, before a later
checkout could remove them. All 16 builds produced the same binary; software
fingerprints, measurement protocols, fixed work and checksums match. All timed
gates accepted with zero steal. Independently applying the committed analyzer
to eight legs per side gives these 99% hierarchical intervals:

| Workload | Observed delta | 99% interval |
| --- | --- | --- |
| Closures | -0.490% | -13.357% to +1.965% |
| Dense tables | +0.187% | -0.748% to +1.290% |
| Iteration | -0.220% | -0.685% to +0.420% |
| Sparse tables | -0.002% | -4.267% to +1.753% |
| Math/table | -0.624% | -3.037% to +0.735% |

Every interval contains zero, meeting the predeclared indistinguishability
criterion, but the wide intervals do not establish precise repeatability. The
last baseline closure leg's median is 26.87% above its side's median; all five
samples in that process are slow. The sparse-table and math/table observations in that pass also shift, but each
workload runs in a separate process. There are 80 workload processes in this
control, 16 per workload; a pass is an aggregation, not one process. Dense tables
and iteration do not show a matching uniform shift.
Retain this process and all samples. Do not reinterpret the workflow's expected
identical-binary skip as proof of stability or retry to obtain narrower results.

Read-only package-log reconstruction reproduces five historical inventory
hashes. The final fingerprint transition is exactly the `zlib1g` upgrade on
2026-10-23 UTC. The measured binary's static ELF dependency list contains libc,
libgcc and the loader, with no direct zlib dependency. This identifies the
inventory change, not a causal explanation of the dense-table finding. No
packages, governor settings or permissions were changed. A post-run policy or
sensor snapshot cannot establish effective frequency or interrupt activity
during the earlier timed windows. Those conditions and process/cache placement
remain unresolved; a future diagnostic should record them during measurement.

The acceptance workflow previously uploaded evidence only for profile and
string modes. [PR #78](https://github.com/slowchop/rustia/pull/78) extends the
existing private 30-day artifact retention to completed acceptance exports and
summaries, including failures. Its exact-commit full pre-push inventory and
independent workflow review passed. All required CI checks passed and merge-commit
auto-merge delivered it as `0d830e201aac802b5dde4b5c171cf0c7e03090b4`. It changes no thresholds, measurements or publication scope.


### 28 October cause investigation

Static runtime/harness review found no per-process random seed or address-based
guest-table ordering in these workloads. Each recorded sample creates a fresh
ReproducibleStandard VM, warms it with one full call, then times another call.
Allocation and collection execute inside that call. Heap/cache locality,
scheduling and effective frequency remain hypotheses rather than findings.

The existing private profile mode was dispatched on trusted main as
[run 40580638855](https://github.com/slowchop/rustia/actions/runs/40580638855).
It failed before timing because the existing `perf_event_paranoid=4` policy
refused hardware counters. Its failed export was successfully downloaded from
CI, verifying PR #78's retention on an actual failed run. No permission or host
setting was changed, and no counter evidence was obtained.

The separate `observe` mode uses read-only, unprivileged process observations
around the unchanged measured binary: user/system CPU time, faults, context
switches, pinned-core interrupt totals and requested clock-state residency.
Sanitized code/heap/stack mapping bases support process-layout investigation;
no process IDs, file paths or device labels are exported. It uses 16 ABBA legs
(eight per side), all five workloads in separate processes, five repetitions
and the existing fixed work, quiet and timed-steal gates. Exports are private
CI artifacts. These observations include startup/warmup; their clock policy
counters are not effective-frequency measurements. They never enter acceptance,
thresholds or graph history. Missing observations and failed legs are retained.


### First unprivileged observation result

[Run 40584219256](https://github.com/slowchop/rustia/actions/runs/40584219256)
completed at reviewed head `6c987adf`. The retained export hash matches its CI
log. Independent reconstruction verifies all 80 invocation records, binary
identity, fixed work/checksums, layouts, child exits and 16 timed gates. The
post-run package inventory matches the initial package fingerprint.

The largest closure process median was +3.112% above the median of the 16
closure process medians,
not the earlier +26.87%. Its child CPU/wall ratio was 0.999878; all invocation
ratios were at least 0.998059. Across the 80 windows there were 53 interrupt and
24 softirq deltas, zero major faults and zero clock-policy transitions. The
slowest closure window had no recorded interrupt or softirq delta. Requested
frequency residency stayed in the exposed 3.8 GHz state; this is not an
effective-frequency measurement. This cohort did not reproduce or explain the
old outlier, and cannot rule out conditions missing from this experiment. The
first helper read the whole executable to hash it just before each launch;
that prefaulting is an observation effect. The follow-up instead verifies bytes
before the quiet gate and after the workload leg, using per-child file tokens
to bind each invocation to the verified inode without reading its contents.

PR #79 merged with every required check passing as `5830b4d14ff8428e6115a3da1f4640f31d293f6b`.
Detailed evidence and corrected notes are retained in private ops PR #3.

### File-lifecycle crossover and independent replay

The old acceptance lane resets scratch state and copies executables anew for
each pair pass. Its five separate workload processes share the side's copied
file within that pass. The first observer built once and reused two files, so
its negative result does not exclude a file-copy/page-cache interaction.

The follow-up `observe` diagnostic requires byte-identical builds and controls
copy lifecycle while keeping launch pathnames, argv lengths, fixed work and
ASLR policy constant. Its first 16 source legs keep ABBA order and cross fresh
copies with one shared reference inode in mirrored FFRR/RRFF blocks: eight legs
per lifecycle, four per source/lifecycle. Each fresh inode stays linked until
the job's confirmation stage. Anonymous file tokens verify identity without
exporting device/inode numbers or private paths.

Selection is predeclared: choose the file from the slowest closure process,
then the fastest process with a distinct file token. Eight independent replay
legs use those exact retained files in ABBA order (four per file). Every replay
uses the same executable and output pathname; initial/replay records are moved
to distinct archive directories afterward. All five workloads are retained as
controls: 120 workload processes across the crossover and replay. No selected
extreme alone establishes a cause; only independent persistence can support a
file-lifecycle finding. Inode identity does not prove physical page-frame
identity. The diagnostic changes no normal acceptance or graph history.


### File-control result and remaining cause boundary

[Run 40591758175](https://github.com/slowchop/rustia/actions/runs/40591758175)
completed at reviewed source `f8533bc6`. Independent archive reconstruction
verified all 120 records, fixed work/checksums, executable hashes, file tokens,
selection and replay order. All 24 timed gates passed with zero steal ticks.
PR #80 merged through required checks as `3a27384c`.

The slowest initial closure leg was 170.945 ns/unit, +5.731% relative to the
median of the 16 initial leg medians (161.679 ns/unit). This is an observed
extreme, not an estimate of an underlying slowdown. Its retained file did not
remain slow: independent replay measured -0.724% versus the initially fast
file, with a 99% interval of [-5.400%, +4.387%]. The initially fast file later
produced a 170.970 ns/unit closure leg. This does not support a persistent
file-specific explanation, and the earlier 26.87% outlier was not reproduced.
The math/table replay differed by -1.340% [-1.815%, -0.572%]; it does not
explain the old closure outlier. All workloads and unsuccessful hypotheses are
retained rather than selecting a favorable result.

Read-only post-run inspection found the runner's threads confined to the
housekeeping CPU set, without threads explicitly pinned to the benchmark
cores. This is an idle snapshot, not evidence about the original timed window.
The current observer records page-aligned mapping bounds, not the actual
initial stack pointer. A controlled process-layout experiment remains a
candidate discriminator, not a known cause. Existing runner security policy
must be respected; no permission change, global ASLR change or privileged
counter run is authorized by this investigation. Private operational snapshots,
raw exports, analysis and the proposed next experiment belong in rustia-ops.
The calibration warning and existing thresholds remain unchanged.
