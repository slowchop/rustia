check-style:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

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

check-benchmark-results:
    node bench/analyze-fixed-samples.js --self-test
    node tools/check-benchmark-results.js
    node tools/check-wasm-gate-results.js
    node bench/render-current-results.js --check
    node bench/render-claims.js --check
    node tools/check-benchmark-site.js

check-examples:
    cargo test -p rustia --examples --no-run

release-preflight:
    just check-style
    just check-release-notes
    just check-crate-metadata
    just check-workflows
    just check-package-files
    just check-publishability
    just check-package-dry-runs
    just check-progress
    just check-project-status
    just check-benchmark-results
    just check-examples
    cargo test --workspace

benchmarks:
    git diff --quiet --ignore-submodules --
    git diff --cached --quiet --ignore-submodules --
    cargo run -p script-bench-harness --release --features all-engines -- --history bench/history/runs.jsonl --commit $(git rev-parse --short HEAD)
    cargo run -p script-bench-harness --release --no-default-features --features mlua-lua54 -- --engines mlua-lua54 --history bench/history/runs.jsonl --commit $(git rev-parse --short HEAD)
    just gen-current-results

gen-current-results:
    cargo run -p script-bench-harness -- --refresh-results
    node bench/render-current-results.js
    node bench/render-claims.js

gen-benchmark-results: gen-current-results
