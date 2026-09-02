# Repository Agent Instructions

## Benchmark policy

- Never run performance benchmarks on the developer's local machine. This includes benchmark examples or binaries, `cargo bench`, timing or profiling scripts, and ad hoc performance comparisons.
- Local compilation, correctness tests, and static validation are allowed, but their durations must not be treated or reported as performance evidence.
- Obtain performance measurements by pushing a branch, opening a pull request, and using the PR's `ci-bench` workflow.
- Only `ci-bench` results may be reported as benchmark evidence or used to update benchmark results and performance claims.
- If `ci-bench` is unavailable, report the benchmark as pending; never substitute a local run.
