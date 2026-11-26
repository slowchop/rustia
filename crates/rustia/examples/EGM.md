# rustia Examples

## Overview

These examples show the facade's supported embedding shapes: owned values and Serde, chunk builders, diagnostics, modules, live object graphs, host callbacks, typed invocation journals, deterministic object methods, hot replacement, and standard-profile mutable Rust objects.

The introductory embedding, chunk-builder, mutable-object, and live-value files start with their purpose, repository-root run command, and expected outcome. Their comments mark the main integration steps.

## Interface

- [first_script.rs](./first_script.rs) is the runnable source for the README and crate Rustdoc walkthrough. After editing it, run `node tools/sync-project-copy.js` from the repository root. Run the example with `cargo run --locked -p rustia --example first_script` and expect `The score is 42`.
- [embedding.rs](./embedding.rs): owned conversion, extension callbacks, Serde, and decision reset.
- [chunk_builder.rs](./chunk_builder.rs): source labels and exact lexical environments.
- [diagnostics.rs](./diagnostics.rs): human-readable error reports.
- [typed_invocation.rs](./typed_invocation.rs) and [game_objects.rs](./game_objects.rs): borrowed per-call host context and staged output records.
- [live_values.rs](./live_values.rs): rooted handles, shared/cyclic table identity, and typed stale-handle rejection after reset.
- [module_resolution.rs](./module_resolution.rs): host-supplied module resolver.
- [transferable.rs](./transferable.rs): move an idle interpreter and retained function together to a native worker. Run it with `cargo run --locked -p rustia --features send --example transferable`.
- [hot_reload.rs](./hot_reload.rs): candidate installation and host-owned replacement.
- [mutable_objects.rs](./mutable_objects.rs): immediate standard-profile Rust payload mutation.
- [hardened_decision.rs](./hardened_decision.rs): correctness-only stateless entry, explicit memory/fuel/record limits, failed-publication check, and automatic reset/reuse.
- [leaf_lifecycle_bench.rs](./leaf_lifecycle_bench.rs) and [live_serde_bench.rs](./live_serde_bench.rs) are timing examples. Do not run them locally; benchmark policy permits evidence only from `ci-bench`.

## Invariants

- Deterministic effects and slot writes are staged and applied by the host only after a successful invocation.
- A standard/live Rust-object mutation is host state and survives a later guest failure.
- A live handle preserves identity but becomes stale after a decision reset.

## Side Effects

Examples may print status or diagnostics and execute guest code. The two `_bench` examples measure elapsed time if run and are intentionally excluded from local verification.

## Verification

From the repository root, `just check-examples` compiles every example and executes embedding, chunk-builder, mutable-object, live-value, game-object, and hardened-decision correctness checks. Release preflight executes the same six examples. The hardened example checks success, record overflow without publication, and reuse after automatic reset. Do not run the timing examples.
