# Validation workflow

Run `just install-hooks` once in each checkout, then push normally. The hook
requires a clean checkout, validates the actual commit being pushed, runs the
full preflight, and checks that neither HEAD nor the checkout changed during it.
It refuses to replace an existing hook. Git can bypass local hooks, so required
CI remains authoritative.

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
failure, cancellation or a skipped stage cannot pass it. Linux native goldens
remain in the workspace stage. Existing Mac, Node, build-matrix and benchmark
checks remain in place.

The refactor retains all 68 previous preflight commands. It adds compiler unit
tests as an early check, plus tests of stage selection and push rejection. No
fixtures, configurations, corpora, examples or package checks were removed.
Evidence-regeneration overrides are rejected during validation. Partial stage
runs never authorize the pre-push guard.

This changes validation scheduling, not runtime performance. Benchmark
measurements continue only on the dedicated CI runner. CI job durations can
show whether scheduling helps; they are not interpreter performance evidence.
