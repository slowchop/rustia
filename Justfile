check-style:
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings

gen-progress:
    node design/render-progress.js

check-progress:
    node design/render-progress.js --check

check-project-status:
    node tools/check-project-status.js

gen-package-files:
    node tools/check-package-files.js

check-package-files:
    node tools/check-package-files.js --check

check-package-dry-runs:
    node tools/check-package-dry-runs.js

gen-publishability:
    node tools/check-publishability.js

check-publishability:
    node tools/check-publishability.js --check

check-release-notes:
    node tools/check-release-notes.js

gen-crate-metadata:
    node tools/check-crate-metadata.js

check-crate-metadata:
    node tools/check-crate-metadata.js --check

check-workflows:
    node tools/check-workflows.js

# Read-only schema validation of a frozen benchmark config; no host, no build.
bench-validate config:
    node tools/run-linux-fixed-benchmark.js validate "{{ config }}"

# Every commit hash a committed benchmark artifact cites must stay reachable.
check-benchmark-commit-reachability:
    node tools/check-benchmark-commit-reachability.js

check-benchmark-results:
    node tools/run-linux-fixed-benchmark.js validate bench/cloud/dedicated-core2-tape-sparse.json
    node tools/check-benchmark-commit-reachability.js
    node tools/check-benchmark-commit-reachability.js --self-test
    node bench/analyze-fixed-samples.js --self-test
    node tools/run-linode-benchmark.js self-test
    node tools/run-linux-matrix-benchmark.js self-test
    node bench/run-memory-supervisor.js self-test
    node bench/analyze-memory-samples.js self-test
    node bench/run-memory-cohort.js self-test
    node bench/publish-memory-analysis.js self-test
    node tools/run-linux-memory-benchmark.js self-test
    node tools/run-wasm-gate.js self-test
    node tools/run-wasm-dispatch-gate.js self-test
    node tools/check-benchmark-results.js
    node tools/check-wasm-gate-results.js
    node tools/render-benchmark-operations-status.js --check
    node tools/check-current-report-summary.js
    node tools/check-grabby-build-results.js
    node bench/render-current-results.js --check
    node bench/render-claims.js --check

# Build the wasm adapter and prove quota/OOM catchability plus decision reset.
test-wasm-memory-limit:
    cargo build --locked --manifest-path bench/wasm/adapter-runner/Cargo.toml --target wasm32-unknown-unknown --release
    node bench/wasm/test-rustia-memory-limit.mjs

# Report which benchmark workloads reach the C2.5a direct array-slot lane.
check-array-slot-coverage:
    cargo clippy --locked -p rustia-compiler --all-targets --features table-write-diagnostics -- -D warnings
    cargo test --locked -p rustia-compiler --features table-write-diagnostics --test array_slot_write_coverage -- --nocapture

check-examples:
    cargo test --locked -p rustia --examples --no-run

release-preflight:
    node tools/run-release-preflight.js

benchmarks:
    @node -e 'console.error("Local benchmark publication is disabled. Measurement happens on the owned host: dispatch the Cross-Engine Matrix or Rustia Memory workflow, or run just bench-history / just bench-pr."); process.exit(2)'

# Validate the isolated memory child/supervisor, cohort analyzer, and publisher locally.
bench-memory-self-test:
    node bench/run-memory-supervisor.js self-test
    node bench/analyze-memory-samples.js self-test
    node bench/run-memory-cohort.js self-test
    node bench/publish-memory-analysis.js self-test
    node tools/run-linux-memory-benchmark.js self-test

# Takes no arguments: the series starts at the first commit that contains an
# engine and ends at HEAD. Pass replace=1 to supersede an existing artifact,
# which is what a change of machine requires, since numbers measured on the old
# box are not comparable with the new one's.
# Rebuild the whole published performance history on the owned benchmark host.
bench-history replace="":
    node tools/run-dedicated-benchmark.js history {{ if replace == "" { "" } else { "--replace" } }}

# Freeze and attest without measuring anything.
bench-history-plan:
    node tools/run-dedicated-benchmark.js plan

# Re-collect and promote the last remote run without measuring again.
bench-history-fetch replace="":
    node tools/run-dedicated-benchmark.js fetch {{ if replace == "" { "" } else { "--replace" } }}

# Clear a lock a killed run left on the benchmark host.
bench-history-unlock:
    node tools/run-dedicated-benchmark.js unlock

# Reports only: no row is recorded and nothing is published. CI runs this per
# pull request; this recipe is the same lane driven by hand.
# Screen the current branch against its merge base on the owned benchmark host.
bench-pr out="pr-screening.md":
    node tools/run-dedicated-benchmark.js pr --out {{ quote(out) }}

# Run it after any change to the machine. Commit the result to adopt it, since
# it changes what every later pull request is judged against.
# Set the pull-request screening thresholds from an A/A run on the bench host.
bench-calibrate out="bench/pr-screening.json":
    node tools/run-dedicated-benchmark.js calibrate --out {{ quote(out) }}

gen-current-results:
    cargo run -p script-bench-harness -- --refresh-results
    node tools/render-benchmark-operations-status.js
    node bench/render-current-results.js
    node bench/render-claims.js

bench-recover-linux-history:
    node tools/recover-linode-linux-history.js
    just gen-current-results

gen-benchmark-results: gen-current-results
