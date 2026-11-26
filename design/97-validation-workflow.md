# Validation workflow

Run `just install-hooks` once in each checkout. For a frozen final commit, use
`just validated-push`: one process requires a clean checkout, records the exact
checked-out branch, commit SHA and remote push URL, runs the local push checks,
rechecks HEAD, clean state and destination, then pushes the captured SHA to the
captured URL's same-name branch. On a branch without an upstream, pass the remote
once (for example, `just validated-push origin`); tracking is established only
after that successful exact push. A differently named upstream is rejected. The
process uses `--no-verify` only for its own final push because it has just run the
same local checks. It creates no reusable receipt or cached authorization.

Ordinary `git push` runs the same checks through the checkout-local hook. The
hook validates the actual commit being pushed and checks that neither HEAD nor
the checkout changed during validation. Installation refuses to replace an
existing hook.

The local checks cover whitespace errors in the commit, Rust formatting, crate
metadata, and generated project copy. They have a shared 50-second limit and
fail the push if a check stalls. The complete correctness and release checks
still run in hosted CI before merge.

History rewrites use the separate `just validated-force-push` route. It reads the
remote branch tip before validation and pushes only with an exact
`--force-with-lease=<ref>:<observed-tip>` after the unchanged-commit check. Raw
Git options, refspecs, deletions and unleased force are not accepted. A network
failure or changed lease requires another local validation.

The temporary first-history bootstrap guard is retained in staging commit
`417828e` for provenance and is not part of the current source. Current pushes
use the exact-commit local checks above. The initial private transfer was verified with Actions disabled. Current pull requests run the hosted correctness checks.

For iteration, `just check-quick` runs formatting, repository/evidence checks,
tool self-tests and all compiler unit tests. This is early feedback, not release
acceptance. The stale freeze diagnostic assertion is covered by that unit suite.

The dispatch-only protected Quick workflow runs the same partition against the
controller-selected source repository and exact commit. Bulk uses the same
protected command inventory for its Quick, workspace and release stages. The
candidate checkout is only the data being checked. The trusted runner accepts
one normalized absolute `RUSTIA_PREFLIGHT_ROOT` for these protected stages and
for the side-effect-free plan listing. It consumes the override before commands
run. Both local push guards remove it from their environment, so they always
validate the checkout whose exact commit they push.

Bulk runs cheap checks first. A gate stops later work after Quick fails. Portable
target builds and differential checks run next. Only then do the full preflight,
native macOS, Node/Wasm and WebKit jobs start. Every candidate job checks the
requested commit before running candidate code.

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
full local preflight inventory: do not run workload profiling locally. Counts establish
path exposure, not elapsed
cost, a regression cause or performance acceptance. Ordinary builds keep the
instrumentation disabled; timing comparisons still belong to private `ci-bench`.
The counters describe this diagnostic build and do not recover exact paths from
previous benchmark binaries or timed legs.

The full preflight retains the existing release checks, compiler unit tests,
and self-tests for stage selection and push rejection. The one-shot push self-test covers clean first push
and upstream setup, exact ordinary and leased pushes, dirty/index/untracked and
detached rejection, failed or mutating local check rejection, Git-environment
isolation, strict option refusal, same-name destination enforcement, post-check
HEAD-race pinning, remote-URL mutation rejection, and preservation of the
ordinary pre-push hook. No
fixtures, configurations, corpora, examples or package checks were removed.
Full preflight rejects evidence-regeneration overrides. Both push paths use the
same checked-in local check script.

No persistent validation receipt is used. A receipt or signing key readable by
the same OS user that runs Git would be forgeable and would create false
confidence. Binding validation and push in one process removes the duplicate
local gate without creating replayable authority. The same user can bypass any
local Git hook; required CI remains the independent trust boundary.

Public CI validates correctness. The [private benchmark bridge](81-benchmark-operations.md#public-source-profile)
is available for separately reviewed comparisons on exact revisions. Public benchmark
automation and publication remain paused.

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
correctness on standard GitHub-hosted runners. Both workflows ran on the `public` event at launch. PR checks include the
full preflight stages, aggregate, native and browser execution, target builds
and differential smoke. Windows native, real Firefox and longer fuzz campaigns
remain manual. The public workflows use read-only permissions and have no private
runner or benchmark access. The private desktop Safari route remains historical.
See the [CI inventory](../.github/public-ci/README.md) and
[platform execution record](96-browser-platform-acceptance.md).

The full inventory checks the history projection and benchmark source reachability.
All retained benchmark measurements are historical. The [launch record](public-source-activation.md) links both passing hosted workflows on the exact launch commit.
