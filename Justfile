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

check-benchmark-results:
    node bench/analyze-fixed-samples.js --self-test
    node tools/run-digitalocean-benchmark.js self-test
    node tools/run-linode-benchmark.js self-test
    node tools/run-linux-matrix-benchmark.js self-test
    node bench/run-memory-supervisor.js self-test
    node bench/analyze-memory-samples.js self-test
    node bench/run-memory-cohort.js self-test
    node bench/publish-memory-analysis.js self-test
    node tools/run-linux-memory-benchmark.js self-test
    node tools/run-vultr-benchmark.js self-test
    node tools/run-scaleway-benchmark.js self-test
    node tools/run-wasm-gate.js self-test
    node tools/run-wasm-dispatch-gate.js self-test
    node tools/check-benchmark-results.js
    node tools/check-wasm-gate-results.js
    node tools/render-benchmark-operations-status.js --check
    node tools/check-current-report-summary.js
    node bench/render-current-results.js --check
    node bench/render-claims.js --check

# Build the wasm adapter and prove quota/OOM catchability plus decision reset.
test-wasm-memory-limit:
    cargo build --locked --manifest-path bench/wasm/adapter-runner/Cargo.toml --target wasm32-unknown-unknown --release
    node bench/wasm/test-rustia-memory-limit.mjs

check-examples:
    cargo test --locked -p rustia --examples --no-run

release-preflight:
    node tools/run-release-preflight.js

benchmarks:
    @node -e 'console.error("Local benchmark publication is disabled: use just bench-linode-matrix-plan CONFIG and just bench-linode-matrix CONFIG."); process.exit(2)'

# One-time private-prompt DigitalOcean authentication; never pass the token as an argument.
bench-do-auth:
    node tools/run-digitalocean-benchmark.js auth

# Read-only validation of the frozen source identities and current DO catalog.
bench-do-plan:
    node tools/run-digitalocean-benchmark.js plan

# Read-only validation of one diagnostic shared-CPU trial profile.
bench-do-trial-plan profile:
    node tools/run-digitalocean-benchmark.js trial-plan "{{ profile }}"

# Run one qualification-only trial; comparison and history publication are disabled.
bench-do-trial profile:
    node tools/run-digitalocean-benchmark.js trial "{{ profile }}"

# Compare the regular and Premium Intel 2-vCPU/4-GiB profiles sequentially.
bench-do-trials:
    node tools/run-digitalocean-benchmark.js trial-matrix

# Run the frozen comparison on one ephemeral Premium Intel Droplet and confirm destruction.
bench-do:
    node tools/run-digitalocean-benchmark.js bench

# Inspect locally owned run state and its corresponding DigitalOcean resource.
bench-do-status:
    node tools/run-digitalocean-benchmark.js status

# Remove only a coordinator lock whose recorded local process no longer exists.
bench-do-unlock:
    node tools/run-digitalocean-benchmark.js unlock

# Idempotently clean up an interrupted run; pass an id when more than one remains.
bench-do-destroy run_id="":
    node tools/run-digitalocean-benchmark.js destroy "{{ run_id }}"

# One-time private-prompt Linode authentication in an isolated ignored state file.
bench-linode-auth:
    node tools/run-linode-benchmark.js auth

# Read-only validation of the selected frozen config, account safety, and dedicated plan.
bench-linode-plan config="":
    node tools/run-linode-benchmark.js plan {{ if config == "" { "" } else { quote(config) } }}

# Validate a frozen cross-language Linux + wasm matrix config without creating a Linode.
bench-linode-matrix-plan config:
    node tools/run-linode-benchmark.js plan "{{ config }}"

# Run and atomically publish one fresh Linux + wasm matrix on an ephemeral dedicated Linode.
bench-linode-matrix config:
    node tools/run-linode-benchmark.js bench "{{ config }}"

# Validate a frozen cross-engine Linux memory cohort without creating a Linode.
bench-linode-memory-plan config:
    node tools/run-linode-benchmark.js plan "{{ config }}"

# Record and atomically publish one cross-engine Linux memory cohort.
bench-linode-memory config:
    node tools/run-linode-benchmark.js bench "{{ config }}"

# Validate the isolated memory child/supervisor, cohort analyzer, and publisher locally.
bench-memory-self-test:
    node bench/run-memory-supervisor.js self-test
    node bench/analyze-memory-samples.js self-test
    node bench/run-memory-cohort.js self-test
    node bench/publish-memory-analysis.js self-test
    node tools/run-linux-memory-benchmark.js self-test

# Run the selected frozen config on one ephemeral dedicated-CPU Linode and confirm deletion.
bench-linode config="":
    node tools/run-linode-benchmark.js bench {{ if config == "" { "" } else { quote(config) } }}

# Freeze one fixed-harness history reconstruction over source-changing Rustia commits.
bench-linode-backfill-freeze start output:
    node tools/freeze-history-backfill.js "{{ start }}" "{{ output }}"

# Validate a frozen fixed-harness history backfill without creating a Linode.
bench-linode-backfill-plan config:
    node tools/run-linode-benchmark.js plan "{{ config }}"

# Build and measure the frozen historical Rustia revisions on one ephemeral Linode.
bench-linode-backfill config:
    node tools/run-linode-benchmark.js bench "{{ config }}"

# Inspect locally owned run state and its corresponding Linode instance.
bench-linode-status:
    node tools/run-linode-benchmark.js status

# Remove only a coordinator lock whose recorded local process no longer exists.
bench-linode-unlock:
    node tools/run-linode-benchmark.js unlock

# Idempotently clean up an interrupted run; pass an id when more than one remains.
bench-linode-destroy run_id="":
    node tools/run-linode-benchmark.js destroy "{{ run_id }}"

# Re-promote one completed fixed-throughput or evidence-only export after local promotion rolled back; never contacts Linode.
bench-linode-promote run_id:
    node tools/run-linode-benchmark.js promote "{{ run_id }}"

# One-time private-prompt Vultr authentication in an isolated ignored state file.
bench-vultr-auth:
    node tools/run-vultr-benchmark.js auth

# Read-only validation of the frozen source identities, account safety, and bare-metal plan.
bench-vultr-plan:
    node tools/run-vultr-benchmark.js plan

# Run the frozen comparison on one ephemeral Vultr bare-metal server and confirm deletion.
bench-vultr:
    node tools/run-vultr-benchmark.js bench

# Inspect locally owned run state and its corresponding Vultr server.
bench-vultr-status:
    node tools/run-vultr-benchmark.js status

# Remove only a coordinator lock whose recorded local process no longer exists.
bench-vultr-unlock:
    node tools/run-vultr-benchmark.js unlock

# Idempotently clean up an interrupted run; pass an id when more than one remains.
bench-vultr-destroy run_id="":
    node tools/run-vultr-benchmark.js destroy "{{ run_id }}"

# One-time private-prompt Scaleway authentication in an isolated ignored state file.
bench-scaleway-auth:
    node tools/run-scaleway-benchmark.js auth

# Read-only validation of the frozen source identities, account safety, and Elastic Metal plan.
bench-scaleway-plan:
    node tools/run-scaleway-benchmark.js plan

# Run the frozen comparison on one ephemeral Scaleway Elastic Metal server and confirm cleanup.
bench-scaleway:
    node tools/run-scaleway-benchmark.js bench

# Inspect locally owned run state and its corresponding Scaleway server and SSH key.
bench-scaleway-status:
    node tools/run-scaleway-benchmark.js status

# Remove only a coordinator lock whose recorded local process no longer exists.
bench-scaleway-unlock:
    node tools/run-scaleway-benchmark.js unlock

# Idempotently clean up an interrupted run; pass an id when more than one remains.
bench-scaleway-destroy run_id="":
    node tools/run-scaleway-benchmark.js destroy "{{ run_id }}"

gen-current-results:
    cargo run -p script-bench-harness -- --refresh-results
    node tools/render-benchmark-operations-status.js
    node bench/render-current-results.js
    node bench/render-claims.js

bench-recover-linux-history:
    node tools/recover-linode-linux-history.js
    just gen-current-results

gen-benchmark-results: gen-current-results
