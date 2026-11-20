# Rustia design drift review, 21 November 2026

Reviewed all 40 tracked Markdown files directly under `design/` against public source [`fad6cc6`](https://github.com/slowchop/rustia/commit/fad6cc6337f5c1731f9b6b7de2951a27a5fea66c), after [#29](https://github.com/slowchop/rustia/issues/29) closed. Findings below refer to that baseline. The accompanying changes correct documentation. Dated receipts and snapshots retain their original results.

The open parser gap is the `type function` declaration. An additional configuration difference affects explicit call type arguments. No runtime change was made in this review.

Verification included 69 parser tests, 32 facade regression tests across six suites, two VM collection-schedule tests and the bounded differential smoke against the pinned Luau 0.709 worker. The worker also ran the same three short programs on both engines:

| Source | Pinned Luau with default flags | Rustia in LuauCompat |
| --- | --- | --- |
| `type function id(x) return x end; return 'ok'` | Returns `ok` | Parse error |
| `local function id<T>(x:T):T return x end; return id<<string>>('ok')` | Parse error | Returns `ok` |
| `local function f() return 1,2 end; return tostring(select('#', f() :: number))` | Returns `1` | Returns `1` |

The project status check verified the manifest, reference and adapter counts. The review used the committed measurements and launch job evidence. It did not run benchmarks, repeat platform validation or establish full language parity.

## 00-constraints.md

No drift detected.

## 02-pillars.md

No drift detected.

## 05-luau-target.md

- The flags-off summary omitted a parser exception. Rustia accepts `id<<number>>(1)`. The default 0.709 oracle rejects it. The [upstream harness](conformance/upstream/luau-0.709/tests/Conformance.test.cpp) enables `LuauExplicitTypeInstantiationSyntax` for the imported case. The target now identifies that exception and distinguishes mlua’s use in the oracle and benchmark adapters from Rustia’s interpreter.

## 10-landscape.md

- The June write-up attributed a 26% difference to host calls. Both the arithmetic representation and the calls changed between variants. Corrected the inference using the [original experiment record](design/10-landscape-raw.json).

## 100-documentation-review.md

No drift detected.

## 15-gapclose.md

- The dispatch comparison characterized tail-call dispatch as tested. The [spike](spikes/spike-b-dispatch/src/lib.rs) called a function pointer and returned to a loop. Corrected the comparison and the architecture summary.

## 30-architecture.md

- The crate map and GC section described Standard as an incremental gc-arena collector driven by allocation debt. The [scheduler](crates/rustia-vm/src/vm.rs) checks executed instructions across profiles, and the [owned heap](crates/rustia-vm/src/heap.rs) marks and sweeps at collection. Standard scripts can stop scheduling collections while the counter keeps advancing. Updated the architecture and kept the earlier spike as history.
- Removed two stale live coverage counts in favor of the [inventory](conformance/COVERAGE.md). Qualified full type-grammar and tail-call-dispatch claims against parser and spike evidence.

## 50-conformance.md

- The historical table claims Rustia erases `type function` declarations. The default oracle accepted `type function id(x) return x end` followed by `return 'ok'`. Rustia rejected the declaration before running the script. **Open parser gap.** Accepting and erasing the declaration is separate from evaluating user-defined types in a type checker.
- The inventory also omitted Rustia’s explicit call type arguments and described incremental Standard collection. Added dated qualifications ahead of the historical rows. The [upstream test](conformance/upstream/luau-0.709/tests/conformance/explicit_type_instantiations.luau) covers the flag-enabled call syntax.

## 55-improvements.md

- The numeric conversion proposal assigned one saturating rule to formatting, buffer integers and `bit32`. The [VM](crates/rustia-vm/src/vm.rs) uses distinct conversions. The document now distinguishes those cases. Qualified thread affinity as the default and marked serialization, rooted function handles and module replacement as shipped.

## 56-performance-research.md

- The comparison table called its July commit the current snapshot. Dated that table and linked the [later claims record](bench/CLAIMS.md). Distinguished proposed Standard GC pacing from the current scheduler.

## 57-memory-benchmarking.md

- The memory workload plan treated seven non-default workloads as the present inventory. The harness has 20 entries, five default. The plan now links the [workload matrix](design/80-bench-suite.md#the-matrix).

## 58-vm-core2.md

- The text gave 33–69 removable clears per workload. Corrected the range to 26–61 using the five values in the [committed census](design/58-vm-core2-c23-census.json).

## 65-wasmjit.md

- The spike extrapolated one numeric-kernel V8 result to production Rustia in a browser and mislabeled a static-Wasm comparator as native. The [raw record](design/65-wasmjit-raw.json) compares a skeleton interpreter, generated Wasm and statically compiled Wasm in Node and Deno. Narrowed the conclusion to those measurements. Native supplied a checksum comparison.

## 70-host-bridge-contract.md

No drift detected.

## 72-host-services.md

No drift detected.

## 75-embedded-api.md

- The API guide identified the hidden value representation as an internal gc-arena value. Reworded that line to match the [VM value type](crates/rustia-vm/src/value.rs).

## 76-koto-migration.md

- The migration guide still deferred userdata generally. The [object API](crates/rustia/src/object.rs) already exposes typed registration, method calls, length and iteration. Linked the separate outstanding scoped-borrow and broader hook work.

## 77-hardened-leaf-environment.md

No drift detected.

## 78-host-functions.md

- The guide categorically excluded live handles from closure captures. Default handles are thread-affine. Under `transferable`, a `Send` handle in synchronized owned storage can meet the closure bounds. Qualified the statement against the [transferable facade tests](crates/rustia/tests/transferable_facade.rs).

## 79-script-diagnostics.md

No drift detected.

## 80-bench-suite.md

- The proposed golden-hash column and Rustia profile were presented as fields in measurement rows. Rows store the workload checksum. The [cross-target transcript](design/84-private-platform-acceptance.md) runs separately. Updated the schema explanation.

## 81-benchmark-operations.md

No drift detected.

## 82-error-behaviour-fuzzing.md

- The prose called all 40 earlier generator templates runtime shapes. Some deliberately generate invalid syntax. The [generator](fuzz/src/differential.rs) adds 21 type-grammar templates, making 61 total. The [smoke test](fuzz/tests/smoke.rs) supplies variant bytes to selected templates and exercises 256 explicit compositions. Corrected the description.

## 83-rust-objects.md

No drift detected.

## 84-private-platform-acceptance.md

- The private baseline described zero, duplicate and failed native transcript fixtures as if they were hosted policy self-tests. The [policy checker](tools/check-workflows.js) selects the hosted path before those fixtures. The native runtime check still enforces the transcript. Clarified the test scope.

## 90-release-readiness.md

No drift detected.

## 91-semver-policy.md

No drift detected.

## 92-first-release.md

- The first-release plan called 726 fixtures and 520 behavior references current. The verified inventory has 734 and 524. Replaced the duplicated count with the [coverage inventory](conformance/COVERAGE.md).

## 93-private-api-acceptance.md

No drift detected.

## 94-release-compatibility-audit.md

- Two summaries pointed readers to closed [#31](https://github.com/slowchop/rustia/issues/31) for remaining work. Updated their chronology and linked the separate platform follow-ups. The old diagnostic results are dated evidence.

## 95-numeric-index-contract.md

No drift detected.

## 96-browser-platform-acceptance.md

- The Firefox discussion attributed dispatch and omitted-execution mutation tests to the active hosted profile. The [hosted self-test](tools/check-public-source-candidate.js) mutates the Firefox checksum. Those other fixtures belong to the earlier private profile. Clarified the coverage.

## 97-validation-workflow.md

- The text said private comparison access was awaiting a trusted bridge. The [operations status](bench/operations-status.json) records availability for separately reviewed exact-cohort runs. Public benchmark automation and publication remain paused.

## 98-public-source-readiness.md

No drift detected.

## 99-benchmark-history-integrity.md

No drift detected.

## CURRENT-2026-10-30-snapshot.md

No drift detected.

## CURRENT.md

- The #18/#19 entry described an early two-fixture milestone and buried the current behavior. Replaced it with the caller-location rule, type erasure and result adjustment, the `type function` gap, and links to the [regressions](crates/rustia/tests/issue_18_19.rs) and [type grammar cases](crates/rustia/tests/type_grammar.rs).
- Corrected the automatic collection schedule and removed the stale native macOS arm64 execution gate. The [launch](design/public-source-activation.md) exercised arm64. Native Intel macOS [#39](https://github.com/slowchop/rustia/issues/39) and remote desktop Safari [#40](https://github.com/slowchop/rustia/issues/40) remain separate.

## EGM.md

No drift detected.

## README.md

No drift detected.

## public-source-activation.md

No drift detected.
