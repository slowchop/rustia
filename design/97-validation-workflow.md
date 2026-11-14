# Validation workflow

Run `just install-hooks` once in each checkout, then push normally. The hook
requires a clean checkout, validates the actual commit being pushed, runs the
full preflight, and checks that neither HEAD nor the checkout changed during it.
It refuses to replace an existing hook. Git can bypass local hooks, so required
CI remains authoritative.

The temporary first-history bootstrap guard is retained in staging commit
`417828e` for provenance and is not part of the current source. Current pushes
use the ordinary exact-HEAD and full-preflight guard above. The initial private
transfer was verified with Actions disabled; accepting staged revisions requires
the CI correctness checks. This does not authorize public visibility.

For iteration, `just check-quick` runs formatting, repository/evidence checks,
tool self-tests and all compiler unit tests. This is early feedback, not release
acceptance. The stale freeze diagnostic assertion is covered by that unit suite.

`just release-preflight` runs the early checks, then workspace tests and release
checks concurrently. It keeps two heavy processes at most, retains the normal
workspace build cache, and gives release checks a separate persistent build
directory. Packaging and rustdoc-dependent scripts remain serialized to avoid
shared archive and documentation races. A cold cache can increase compilation
work; no speedup is claimed until observed.

CI runs the three stages independently. Every stage fetches the pinned lockfile
dependencies it needs, including offline package consumers. The unchanged
required status `Release preflight` succeeds only when all stages succeed;
failure, cancellation or a skipped stage cannot pass it. The gate also requires
the headless WebKit browser job; real Safari remains release-candidate
acceptance. The WebKit job runs in a container and adds only `$GITHUB_WORKSPACE`
to Git's safe-directory list before the browser tests, because the checkout
action's temporary HOME setting does not carry into that container step. Linux
native goldens remain in the workspace stage. Mac, Node and build-matrix checks
remain in place. Saved benchmark artifacts are validated offline; the private
staging profile runs no performance measurements.

The Linux workspace job also checks the default-off `executor-diagnostics`
feature and runs the fixed `gc_accounting_paths` test with its captured output.
That CI-only diagnostic partitions root-refresh attempts by control-memory
accounting state and reports closure/leaf executor counts separately for setup,
one full-N warmup and the following call on the same VM. It is outside the local
pre-push command inventory: do not run workload profiling locally. Counts establish
path exposure, not elapsed
cost, a regression cause or performance acceptance. Ordinary builds keep the
instrumentation disabled; timing comparisons still belong to private `ci-bench`.
The counters describe this diagnostic build and do not recover exact paths from
previous benchmark binaries or timed legs.

The refactor retains all 68 previous preflight commands. It adds compiler unit
tests as an early check, plus tests of stage selection and push rejection. No
fixtures, configurations, corpora, examples or package checks were removed.
Evidence-regeneration overrides are rejected during validation. Partial stage
runs never authorize the pre-push guard.

This changes validation scheduling, not runtime performance. Benchmark
measurement is paused in the active profile pending the reviewed trusted private
bridge; retained private procedures do not authorize a new run. CI job durations
can show whether scheduling helps; they are not interpreter performance evidence.

## First CI observation

[Run 39883557562](https://github.com/slowchop/rustia-historic/actions/runs/39883557562)
passed the preflight gate in 20 minutes 21 seconds from the first stage start to
the aggregate completion. The previous successful main preflight,
[39863764500](https://github.com/slowchop/rustia-historic/actions/runs/39863764500), took
41 minutes 4 seconds. This is about 50% less elapsed time in this pair of runs,
not a controlled comparison: machine state, caches and revisions differ.
The quick stage finished in 41 seconds including setup. All three stages passed.
The overall manual run failed separately during Windows Rust startup, before
Windows correctness tests executed; this observation does not close that gate.

## Prepared public source profile

An explicit public-source marker selects private self-hosted correctness CI while retaining
the full preflight stages and aggregate gate. Benchmark automation is paused;
existing measurements are historical records. The full inventory additionally
checks the history projection and refuses missing benchmark source identifiers.
All jobs use ordinary self-hosted groups with private-repository and trusted
same-repository PR guards; the final aggregate retains its always-run failure
check within that trusted scope. Windows native and real Firefox execution stay manual. Windows requires an
online Windows runner; Firefox uses the existing Linux Docker correctness pool
with checksum-pinned official browser/driver downloads inside its disposable
container. The optional Firefox job does not replace the required WebKit gate. An additional
private Safari job requires explicit `safari=true` dispatch, an exact installed
browser version and an already logged-in runner desktop. It checks prerequisites
without changing browser/OS permissions and is skipped by routine PR/main CI.
The optional real-browser jobs do not weaken the required aggregate gate. Public visibility needs a separate isolated
execution plan. No hosted fallback or benchmark-runner access is added.
This does not authorize a package release or replace platform release gates.
