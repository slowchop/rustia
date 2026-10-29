# EGM Drift Report — 2026-10-29

Target: the new `slowchop/rustia` repository, baseline `b84b5f6715e31756f00d1f39e72c8246f179b174`.
The retired checkout was excluded. Fixes are isolated on `codex/egm-drift-fixes`.

Scope: all 37 Markdown design documents, their named current implementation/test boundaries,
related source maps and contributor/benchmark/fuzz guides. Every tracked Markdown file was
checked for relative file targets. This is a static drift audit, not a proof of every VM path,
a new oracle campaign, a fresh external-client/platform acceptance, or a re-audit of every raw
historical receipt. Explicit historical research, disabled experiments and acknowledged release
gaps remain historical or pending. No benchmarks or profiling ran. No runtime semantics,
measurement values, thresholds, package publication or repository visibility were changed.

“Fixed” records an applied correction to an inaccurate current claim or unusable source reference.
Open memory-protocol requirements are retained below; documentation corrections do not implement them.
Historical PR/Actions hyperlinks in the affected designs now target `slowchop/rustia-historic`,
which owns those receipts. Those private links are not anonymously accessible verification.

## design/00-constraints.md

No drift detected.

## design/02-pillars.md

No drift detected.

## design/05-luau-target.md

No drift detected.

## design/10-landscape.md

No drift detected.

## design/15-gapclose.md

No drift detected.

## design/30-architecture.md

- Fixed: GC/build-plan prose still described ordinary side stores as append-only and reclamation/accounting as missing. `crates/rustia-vm/src/heap.rs` reclaims ordinary objects; `vm.rs` reclaims unreachable programs and exposes quiescent collection. Pacing and bounded-pause tuning remain open.
- Fixed: the build plan called the fuzz harness, transactional installation, live constructors and live Serde future work. Those paths exist in `fuzz/`, `crates/rustia/src/lib.rs` and `live_value.rs`, with focused regressions. Independent-frontend comparison and broader release gates remain planned.
- Fixed: universal NaN ingress wording omitted exact buffer-bit reads, and the profile summary omitted deterministic clock-provider admission. Source is `crates/rustia-vm/src/value.rs`, `vm.rs` and `tests/core.rs`; arithmetic canonicalization and deterministic policy remain intact.

## design/50-conformance.md

- Fixed: authoritative corrections still promised capability-error stubs for removed environment globals and unconditional `os` removal. Ordinary deterministic VMs retain `getfenv`/`setfenv`; hardened leaves omit them, and deterministic clock providers may admit `os`. Source is `crates/rustia-vm/src/vm.rs` and `extension.rs`.
- Fixed: the PUC appendix described `pc N:` as universal despite the source-aware renderer in `crates/rustia-vm/src/error.rs` and four-profile diagnostic regressions. Low-level source-free programs retain the PC fallback.
- Fixed: the current semantic-tail introduction still marked resumable module loading and transactional installation as awaiting final verification; it now links scoped acceptance in `design/93-private-api-acceptance.md`. The older 709-row inventory remains historical. The ABI-v3 row's stale final-review gate was also updated to its scoped private acceptance.

## design/55-improvements.md

- Fixed: improvement 8 said `buffer.readf64` canonicalized payloads and generalized negative-zero normalization. `Value::number_from_bits` preserves deterministic buffer input bits; native and bare-Wasm regressions cover f32/f64 round trips. `string.unpack` still uses ordinary canonicalized numeric ingress.
- Fixed: improvement 10 named nonexistent `.luau_message()` and removed-builtin capability stubs. The implemented VM renderer is `Error::message()` plus source-aware `Display`; capability errors concern rejected host registrations. Broader diagnostic parity remains explicitly scoped.

## design/56-performance-research.md

- Fixed: current-plan prose treated bounded memory/reset and O2 machinery as future work. Lifecycle support and default-disabled O2 families/logical receipts exist; clear elision, default enablement measurements and production wasm/JIT equivalence remain gated. No performance estimate or measurement changed.

## design/57-memory-benchmarking.md

- Fixed: current capability wording implied an active memory workflow. The explicit source profile has hosted correctness/artifact checks only; fresh memory evidence remains pending a trusted private bridge.
- Fixed metric description: `bench/analyze-memory-samples.js` publishes raw quiescent-marker anonymous memory minus warmed anonymous memory, without null-control adjustment. Its area metric integrates excess cgroup current charge above the spawned baseline between warmed and quiescent markers, rather than anonymous-memory area.
- Open implementation requirements: null-control-adjusted retention and anonymous-memory area remain absent. `bench/run-memory-cohort.js` also lacks the separately required warmup process, and `bench/src/main.rs` emits the quiescent marker immediately after the final epoch without a bounded idle/collection opportunity. These requirements remain explicit; prospective protocol versioning and fresh accepted evidence are required before claiming them. Historical values, thresholds and acceptance results were not recalculated or relabeled.

## design/58-vm-core2.md

- Fixed: current measurement instructions still prescribed retired DigitalOcean lifecycle operations. They now require the owned-host protocol and respect this checkout's paused source profile; historical cloud results remain intact.
- Fixed: caller proof, unary math, scalar metadata and tape handoff still carried pending acceptance claims after the final combined PR #74 candidate was accepted in `design/95-numeric-index-contract.md`. No isolated subchange receives credit for the full combined result.
- Fixed: zero direct-array-slot hits were described as the current full-suite result. That is the initial 17-workload census before `array_mutation`; its positive-coverage regression is `crates/rustia-compiler/tests/array_slot_write_coverage.rs`.

## design/65-wasmjit.md

No drift detected.

## design/70-host-bridge-contract.md

No drift detected.

## design/72-host-services.md

- Fixed: the present-tense 712-fixture/511-ref total was stale. Current counts are 726 fixtures / 520 refs; the document now links `conformance/COVERAGE.md` instead of copying a moving total.

## design/75-embedded-api.md

- Fixed: object review, controlled-client integration and the common private candidate were still pending despite the accepted record in `design/93-private-api-acceptance.md` and implemented object/invocation contracts.
- Fixed: the build order said stack traces remained absent. Bounded logical call trails are implemented and tested in `crates/rustia/src/diagnostic.rs`. Wider API freeze, custom fields/metamethods, scoped and async layers remain open.

## design/76-koto-migration.md

No drift detected.

## design/77-hardened-leaf-environment.md

- Fixed: the document counted three `Profile` variants and called the hardened surface a fourth. There are four variants including `Puc51Compat`; `HardenedLeaf` adds no variant (`crates/rustia/src/profile.rs`).

## design/78-host-functions.md

- Fixed: typed-object review and civfun rollout were still pending despite the later accepted implementation/integration record. Historical synchronous-function evidence and deferred async/scoped features remain separate.

## design/79-script-diagnostics.md

- Fixed: historical PR/Actions hyperlinks pointed at the newly created repository, which does not own those receipts. They now target `slowchop/rustia-historic`; original source identities and evidence are unchanged.

## design/80-bench-suite.md

- Fixed: current pipeline prose described active per-commit, PR, matrix, acceptance and memory workflows. All five linked workflow files are absent in this hosted profile. Links now point to the retained coordinator and explicit profile; fresh measurement/publication gates remain pending.

## design/81-benchmark-operations.md

- Fixed: the graph follow-up was still described as local and awaiting delivery. `design/99-benchmark-history-integrity.md` records its predecessor-repository PR #77 delivery and live verification. That historical deployment does not activate this checkout's paused pipeline.

## design/82-error-behaviour-fuzzing.md

- Fixed: builtin follow-up CI/review and yieldable `require` were still pending. The accepted record and module-resolution ABI v3 cover those delivered slices; `crates/rustia/tests/yieldable_require.rs` exercises the lifecycle. Broader language parity remains open.

## design/83-rust-objects.md

- Fixed: the closing evidence section contradicted the delivered header by leaving civfun integration, CI/reviews, yieldable module loading and private-candidate acceptance open. It now points to the exact-candidate evidence while retaining wider public-release gates.

## design/84-private-platform-acceptance.md

- Fixed: the final paragraph still treated the immutable private candidate and active-client rollout as future evidence. Those records exist in `design/93-private-api-acceptance.md`; the wider platform matrix remains open.

## design/90-release-readiness.md

- Fixed: current operations prose claimed active measurement/publication and a CI contents-write grant that this hosted profile forbids. Fresh screening stays pending the trusted bridge.
- Fixed: Linux ARM64 runtime evidence and delivered diagnostic/index checks were still entirely pending. Document 96 records exact-runtime ARM64 shared contracts; Intel Mac/mobile and final-candidate/full-matrix acceptance remain required.

## design/91-semver-policy.md

No drift detected.

## design/92-first-release.md

- Fixed: fresh measurement instructions pointed at absent remote benchmark workflows. The final-candidate evidence requirement remains; measurement stays pending until the reviewed private bridge exists.

## design/93-private-api-acceptance.md

- Fixed: the work-left paragraph still required method-lookup order and yielded-prefix fixes subsequently closed by document 94 and their in-tree regressions. Historical baseline hashes, counts and client pins remain unchanged.

## design/94-release-compatibility-audit.md

- Fixed: the headline and delivery gates lagged closeout PR #72, diagnostic PR #73 and accepted numeric-index/runtime PR #74. They now distinguish delivered scoped work from the wider public-release parity audit.

## design/95-numeric-index-contract.md

- Fixed: historical PR/Actions hyperlinks pointed at the newly created repository, which does not own those receipts. They now target `slowchop/rustia-historic`; original source identities and evidence are unchanged.

## design/96-browser-platform-acceptance.md

- Fixed: real Firefox/Safari instructions claimed that `browser-correctness.yml` remained dispatchable here. Only hosted correctness/fuzz workflows exist; the local explicit-browser correctness runner remains, and routine PR CI uses headless WebKit. Real-browser RC evidence and remote runner readiness remain open.

## design/97-validation-workflow.md

- Fixed: the validation summary said measurement continued on the dedicated runner despite this profile's explicit pause. Hosted correctness and artifact validation remain active; benchmark evidence stays pending.

## design/98-public-source-readiness.md

No drift detected.

## design/99-benchmark-history-integrity.md

- Fixed: historical PR/Actions hyperlinks pointed at the newly created repository, which does not own those receipts. They now target `slowchop/rustia-historic`; original source identities and evidence are unchanged.

## design/CURRENT.md

- Fixed: the action queue still asked to finish implemented stack diagnostics; current benchmark instructions and the closing summary implied active automation. Predecessor pipeline descriptions are now explicitly historical.
- Fixed: canonical speed prose copied obsolete ratios and still listed reclamation/quotas as future work. Current ratios now come from generated `bench/CLAIMS.md`; historic comparisons remain untouched.
- Added the four unmet memory-protocol requirements and the dated drift-audit result so the canonical state agrees with document 57. Main protection/auto-merge settings were checked as disabled on 2026-10-29; no settings were changed.

## design/EGM.md

No drift detected.

## design/README.md

- Fixed: the index used obsolete current conformance counts, stale private-delivery gates and active benchmark wording, and omitted documents 95–99. It now distinguishes accepted private work, retained measurements and open public-release/protocol gates.

## README.md

- Fixed: the Cargo dependency pinned historical `92a85d65...`, absent from the rewritten repository. It now pins reachable cleaned-source baseline `b84b5f6715e31756f00d1f39e72c8246f179b174`.
- Fixed: three immutable evidence URLs used absent pre-projection IDs. They now use mapped source commits while measurement IDs and data remain original.
- Added source-reference regression checks to `tools/check-readme-comparisons.js`: invalid/unreachable pins and absent immutable source targets fail independently of the Rust code-fence tests.

## AGENTS.md

- Fixed: repository guidance asserted that main rulesets, auto-merge and disabled squash/rebase settings were already enforced. The new private repository currently has no main protection and auto-merge is disabled; merge-commit/validation/review policy remains required.
- Fixed: benchmark guidance now explicitly states that this source profile pauses measurement and publication pending a trusted private bridge.

## CONTRIBUTING.md

- Fixed: benchmark instructions implied an available PR measurement check in this checkout. They now preserve the no-local-benchmark rule and leave fresh measurements pending the trusted bridge.

## bench/README.md

- Fixed: measurement examples and publication/memory workflow descriptions implied an active local/remote path here. They are now scoped to the authorized predecessor benchmark host; local work remains offline analysis and correctness checks.

## fuzz/README.md

- Fixed: `run-error-fuzz` PR-label campaigns were documented despite the hosted workflow supporting long campaigns only by explicit dispatch.

## EGM.md

- Fixed: the root map now states the hosted source profile and paused automation, and links this dated report.

## tools/EGM.md

- Fixed: main-series publication and 30-day acceptance-artifact workflow claims were not scoped to predecessor private operations. The map now identifies their historical scope and the README source-reference validation boundary.
