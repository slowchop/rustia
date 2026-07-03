gen-progress:
    node design/render-progress.js

gen-benchmark-results:
    cargo run -p script-bench-harness -- --refresh-results
    node bench/render-current-results.js
