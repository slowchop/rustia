gen-progress:
    node design/render-progress.js

benchmarks:
    git diff --quiet --ignore-submodules --
    git diff --cached --quiet --ignore-submodules --
    cargo run -p script-bench-harness --release --features all-engines -- --history bench/history/runs.jsonl
    cargo run -p script-bench-harness --release --no-default-features --features mlua-lua54 -- --engines mlua-lua54 --history bench/history/runs.jsonl
    just gen-benchmark-results

gen-benchmark-results:
    cargo run -p script-bench-harness -- --refresh-results
    node bench/render-current-results.js
