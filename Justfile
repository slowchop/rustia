gen-progress:
    node design/render-progress.js

check-progress:
    node design/render-progress.js --check

gen-package-files:
    node tools/check-package-files.js

check-package-files:
    node tools/check-package-files.js --check

check-release-notes:
    node tools/check-release-notes.js

check-benchmark-results:
    node tools/check-benchmark-results.js
    node bench/render-current-results.js --check

release-preflight:
    just check-release-notes
    just check-package-files
    just check-progress
    just check-benchmark-results
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

gen-benchmark-results: gen-current-results
