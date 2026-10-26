# EGM Drift Report — 2026-10-26

Scope: all 68 generated `EGM.md` files in the eligible source tree. This is a static map-to-source and referenced-contract audit, not an exhaustive behavioral or release audit. Every map was read; source claims, file links, child-map links, and verification targets were checked. Tests, fixture-only trees, vendored/generated/ignored trees, symlinks, and hidden configuration directories were excluded from bootstrap.

Map corrections below are applied. The initial bootstrap preserved authoritative design prose, added map-reading/maintenance instructions to `AGENTS.md`, and refreshed the generated `design/package-files.json` inventory for newly packaged maps. The authorized documentation follow-up now resolves all three reported disagreement themes: conformance counts, the fuzzing CI claim, and benchmark operations. `tools/EGM.md` was updated to remove its obsolete disagreement note. Application code and human-facing READMEs remain unchanged. No benchmarks or runtime tests were run during the static map audit; subsequent PR validation runs in CI.

## CI integration follow-up

- Integrated the benchmark publication already on `main` and regenerated the combined package inventory: the benchmark package contains 534 files, including the new maps and published evidence. Refreshed the dependent `design/publishability.json` counts.
- Regenerated `bench/operations-status.json` and its HTML display from the merged Git history. The existing path-based policy counts maps under crate `src/` directories as relevant commits; the pending status does not indicate an application-code change.
- Package, publishability, project-status, benchmark-artifact, and report checks validate the generated outputs. Existing measurements, application code, and human-facing READMEs remain unchanged relative to `main`.

## EGM.md

- Corrected the browser-demo boundary: it uses the standard preset with stdout disabled, not the live-services preset ([runner source](demos/live-wasm/wasm-runner/src/lib.rs), line 7).
- No remaining map drift detected after correction.

## bench/EGM.md

No drift detected.

## bench/grabby-builds/EGM.md

No drift detected.

## bench/src/EGM.md

No drift detected.

## bench/src/bin/EGM.md

- Completed the omitted side effects: the probe reads Linux process-memory files, times construction/execution, and emits JSON samples ([source](bench/src/bin/vm-string-memory.rs), functions `run`, `collect_sample`, and `process_memory_snapshot`).
- No remaining map drift detected after correction.

## bench/wasm/EGM.md

No drift detected.

## bench/wasm/adapter-runner/EGM.md

No drift detected.

## bench/wasm/adapter-runner/src/EGM.md

No drift detected.

## bench/workloads/EGM.md

No drift detected.

## bench/workloads/javascript/EGM.md

No drift detected.

## bench/workloads/lua54/EGM.md

No drift detected.

## bench/workloads/luau/EGM.md

No drift detected.

## crates/EGM.md

No drift detected.

## crates/rustia/EGM.md

No drift detected.

## crates/rustia-cli/EGM.md

No drift detected.

## crates/rustia-cli/src/EGM.md

No drift detected.

## crates/rustia-compiler/EGM.md

No drift detected.

## crates/rustia-compiler/src/EGM.md

No drift detected.

## crates/rustia-conformance/EGM.md

No drift detected.

## crates/rustia-conformance/src/EGM.md

No drift detected.

## crates/rustia-lexer/EGM.md

No drift detected.

## crates/rustia-lexer/src/EGM.md

- Corrected the return type to the actual one-parameter `Result<Vec<Token>>` alias, whose error is `LexError` ([source](crates/rustia-lexer/src/lib.rs), lines 111–115).
- No remaining map drift detected after correction.

## crates/rustia-parser/EGM.md

No drift detected.

## crates/rustia-parser/src/EGM.md

No drift detected.

## crates/rustia-vm/EGM.md

No drift detected.

## crates/rustia-vm/src/EGM.md

No drift detected.

## crates/rustia-vm/src/vm/EGM.md

No drift detected.

## crates/rustia/examples/EGM.md

No drift detected.

## crates/rustia/src/EGM.md

No drift detected.

## demos/EGM.md

No drift detected.

## demos/live-wasm/EGM.md

No drift detected.

## demos/live-wasm/wasm-runner/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## demos/live-wasm/wasm-runner/src/EGM.md

No drift detected.

## design/EGM.md

No drift detected.

## fuzz/EGM.md

No drift detected.

## fuzz/fuzz_targets/EGM.md

No drift detected.

## fuzz/src/EGM.md

No drift detected.

## fuzz/src/bin/EGM.md

- Corrected the worker input bound: it accepts 16 KiB and reads one extra byte to reject oversized input ([source](fuzz/src/bin/error_oracle_worker.rs), lines 6–7).
- No remaining map drift detected after correction.

## spikes/EGM.md

No drift detected.

## spikes/alt-luau-wasm/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/alt-luau-wasm/src/EGM.md

No drift detected.

## spikes/bench-suite/EGM.md

No drift detected.

## spikes/bench-suite/koto-harness/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/bench-suite/koto-harness/src/EGM.md

No drift detected.

## spikes/bench-suite/luau/EGM.md

No drift detected.

## spikes/bench-suite/src/EGM.md

No drift detected.

## spikes/spike-a-luau/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/spike-a-luau/src/EGM.md

No drift detected.

## spikes/spike-b-dispatch/EGM.md

No drift detected.

## spikes/spike-b-dispatch/src/EGM.md

No drift detected.

## spikes/spike-b-value/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/spike-b-value/src/EGM.md

No drift detected.

## spikes/spike-c-libm/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/spike-c-libm/src/EGM.md

No drift detected.

## spikes/spike-d-gc/EGM.md

No drift detected.

## spikes/spike-d-gc/src/EGM.md

No drift detected.

## spikes/spike-ga-gcarena/EGM.md

- Corrected `prove_not_send` from an optional passing check to an expected compile-fail probe. It deliberately asserts that the raw arena implements `Send` ([source](spikes/spike-ga-gcarena/src/main.rs), lines 72–74).
- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/spike-ga-gcarena/src/EGM.md

No drift detected.

## spikes/spike-h-slice/EGM.md

No drift detected.

## spikes/spike-h-slice/src/EGM.md

No drift detected.

## spikes/spike-sk-rustia/EGM.md

No drift detected.

## spikes/spike-sk-rustia/src/EGM.md

No drift detected.

## spikes/spike-snap/EGM.md

- Added the missing link to the immediate child `src/EGM.md`; the initial map linked source files but left the child map out of the navigation tree.
- No remaining map drift detected after correction.

## spikes/spike-snap/src/EGM.md

No drift detected.

## spikes/spike-wasmjit/EGM.md

No drift detected.

## spikes/spike-wasmjit/src/EGM.md

No drift detected.

## tools/EGM.md

- Expanded the coordinator interface to include per-commit, PR screening, acceptance, live-Serde, and calibration operations, and distinguished package-audit generation from read-only checking ([coordinator](tools/run-dedicated-benchmark.js), `parseArgs`; [package audit](tools/check-package-files.js)).
- The follow-up replaced the obsolete design-disagreement note with links to the reconciled owned-host contracts.
- No remaining map drift detected after correction.

## tools/puc51-oracle/EGM.md

No drift detected.

## design/CURRENT.md

- Resolved: the known-gaps opening and current action queue now agree with the [manifest](conformance/MANIFEST.tsv): 726 fixtures (708 Luau 0.709 and 18 PUC 5.1.5), and the [catalog](conformance/REFS.tsv): 520 refs (502 Luau and 18 PUC). The [runner](crates/rustia-conformance/src/lib.rs), `run_repository_with_experimental_o2`, derives those counts from the files. Explicitly historical counts remain historical.
- Resolved: the canonical doc-map row now identifies implemented owned-host workflows and the retired cloud lifecycle.
- No remaining drift detected for the reported disagreements.

## design/30-architecture.md

- Resolved: the live-state summary and current profile denominator now use 726 fixtures / 520 refs, including the correct Luau and PUC splits, matching the [manifest](conformance/MANIFEST.tsv) and [catalog](conformance/REFS.tsv). Historical design-row counts remain unchanged.
- No remaining drift detected for the reported disagreement.

## AGENTS.md

- Resolved: principle 5 now names bounded frontend/VM smoke checks and the pinned Luau 0.709 differential oracle. It links the [fuzz harness](fuzz/README.md) and [CI workflow](.github/workflows/error-fuzz.yml), which builds the oracle worker and runs `generated_differential_smoke`. Independent-frontend comparison such as `full_moon` is explicitly planned, consistent with the [architecture](design/30-architecture.md) and current dependencies.
- No remaining drift detected for the reported disagreement.

## design/80-bench-suite.md

- Resolved: current tracking and automation now name the owned `ci-bench` host, its [coordinator](tools/run-dedicated-benchmark.js), and the implemented per-commit, PR, weekly-matrix, acceptance, and memory workflows. The contradictory assertion that no measurement workflow exists was removed. The current engine/workload inventory now includes separate Luau interpreter/JIT modes and the 19 source-covered workload entries in [the harness](bench/src/main.rs).
- Resolved: cloud results and their retired recipes are explicitly historical; `just bench-vultr` is no longer presented as available. Current A/B acceptance links to the [owned-host contract](design/81-benchmark-operations.md#ab-acceptance-on-the-owned-host) and is distinguished from historical three-run publication rules. Existing historical measurements were not recomputed or promoted.
- No remaining drift detected for the reported disagreement.
