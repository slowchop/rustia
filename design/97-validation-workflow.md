# Validation workflow

Run `just install-hooks` once in each checkout. For a frozen final commit, use
`just validated-push`: one process requires a clean checkout, records the exact
checked-out branch, commit SHA and remote push URL, runs the full preflight,
rechecks HEAD, clean state and destination, then pushes the captured SHA to the
captured URL's same-name branch. On a branch without an upstream, pass the remote
once (for example, `just validated-push origin`); tracking is established only
after that successful exact push. A differently named upstream is rejected. The
process uses `--no-verify` only for its own final push because it has just run the
same full validation. It creates no reusable receipt or cached authorization.

Ordinary `git push` remains supported and still runs the full checkout-local
hook. The hook validates the actual commit being pushed and checks that neither
HEAD nor the checkout changed during validation. Installation refuses to replace
an existing hook. Git can bypass local hooks, so required CI remains authoritative.

History rewrites use the separate `just validated-force-push` route. It reads the
remote branch tip before validation and pushes only with an exact
`--force-with-lease=<ref>:<observed-tip>` after the unchanged-commit check. Raw
Git options, refspecs, deletions and unleased force are not accepted. A network
failure or changed lease requires another complete validation.

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
tests as an early check, plus tests of stage selection and push rejection. The
one-shot push adds its own offline self-test to that inventory: clean first push
and upstream setup, exact ordinary and leased pushes, dirty/index/untracked and
detached rejection, failed or mutating preflight rejection, Git-environment
isolation, strict option refusal, same-name destination enforcement, post-check
HEAD-race pinning, remote-URL mutation rejection, and preservation of the
ordinary full hook. No
fixtures, configurations, corpora, examples or package checks were removed.
Evidence-regeneration overrides are rejected during validation. Partial stage
runs never authorize either push path.

No persistent validation receipt is used. A receipt or signing key readable by
the same OS user that runs Git would be forgeable and would create false
confidence. Binding validation and push in one process removes the duplicate
local gate without creating replayable authority. The same user can bypass any
local Git hook; required CI remains the independent trust boundary.

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

## Experimental source release

The [source profile](public-source-profile.json) selects `public-hosted`
correctness on standard GitHub-hosted runners. The `public` event starts both
required workflows when the repository becomes public. PR checks include the
full preflight stages, aggregate, native and browser execution, target builds
and differential smoke. Windows native, real Firefox and longer fuzz campaigns
remain manual. The public workflows use read-only permissions and have no private
runner or benchmark access. The private desktop Safari route remains historical.
See the [CI inventory](../.github/public-ci/README.md) and
[platform execution record](96-browser-platform-acceptance.md).

The full inventory checks the history projection and benchmark source reachability.
All retained measurements are historical. Inspect the actual launch jobs for
their exact revision before recording hosted acceptance.
