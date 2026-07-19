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
    node tools/run-digitalocean-benchmark.js self-test
    node tools/run-linode-benchmark.js self-test
    node tools/run-vultr-benchmark.js self-test
    node tools/run-scaleway-benchmark.js self-test
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

# Read-only validation of the frozen source identities, account safety, and dedicated plan.
bench-linode-plan:
    node tools/run-linode-benchmark.js plan

# Run the frozen comparison on one ephemeral dedicated-CPU Linode and confirm deletion.
bench-linode:
    node tools/run-linode-benchmark.js bench

# Inspect locally owned run state and its corresponding Linode instance.
bench-linode-status:
    node tools/run-linode-benchmark.js status

# Remove only a coordinator lock whose recorded local process no longer exists.
bench-linode-unlock:
    node tools/run-linode-benchmark.js unlock

# Idempotently clean up an interrupted run; pass an id when more than one remains.
bench-linode-destroy run_id="":
    node tools/run-linode-benchmark.js destroy "{{ run_id }}"

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
    node bench/render-current-results.js
    node bench/render-claims.js

gen-benchmark-results: gen-current-results
