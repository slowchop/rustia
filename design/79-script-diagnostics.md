# Human-friendly script diagnostics

Status: implementation in progress on `codex/human-friendly-diagnostics`.
This document does not mark the rollout or release gate complete.

## Contract

The host report explains script failures using the exact executed source,
an honest expression span (or line-only fallback), and a bounded logical call
trail. Guest error values, message strings, protected-call behavior, fuel,
and location suppression remain language semantics, not presentation policy.
Rust backtraces remain conventional function/file/line reports.

`Error` owns an `ErrorKind` and exposes `as_vm_error`, `source_location`,
`source_span`, `diagnostic`, and `report`. `Error::from` wraps frontend/core
failures; `Error::embed` is the concise message constructor and
`Error::external` preserves a typed host cause. `with_context` must preserve
the original typed classification. `Display` stays concise; clients opt into
the shared report at their existing reporting boundaries. Caught errors do
not log automatically.

`DiagnosticLimits` defaults to 4 MiB of source-retention charges per VM,
256 KiB per source, 32 frames, and 16 KiB rendered output. Charges include
snapshot bookkeeping and remain outstanding while errors hold snapshots.
Host-owned compiled artifacts have their own lifetimes; the VM admits its
retention separately. Source retention is best effort, independent of the
guest heap/fuel quotas, and never reads the filesystem during formatting.
Hardened leaves do not capture rich frames or retain diagnostic source.

`SourceMap::generated_prefix` and `generated_wrapper` map single-line
generated text around console input. They do not alter guest source names,
line semantics, or execution. This is deliberately not a general transpiler
source-map API. Invalid wrapper ranges fail before compilation, and failures
inside generated text must not be attributed to the user's input.

## Current implementation checkpoint

- Compiler span annotations preserve the pre-existing guest line map.
- The VM captures a bounded, handle-free call trail before stackless unwind.
- Immutable source snapshots have independent VM retention leases that
  survive reset and VM destruction with outstanding errors.
- The facade uses a typed error object and bounded plain-text reports.
- Parse/compile failures have source identity and snapshots; console wrapper
  mappings are implemented on both ordinary and typed chunk builders.
- The bundled CLI consumes the shared report, fixing its prior assumption
  that every span belonged to the latest REPL input.
- Failing index/call/arithmetic operations provide factual type explanations.
  Parser-proven unclosed blocks name their opening line and missing keyword.
- Serde conversion paths use Lua's one-based sequence positions; conversion
  failures identify the returned-value boundary without inventing source spans.
- Module resolvers preserve typed errors, dependency source, and loading callers.
  Host causes remain downcastable and render through a bounded cause chain.
- Failed installations state the guest rollback guarantee and explicitly exclude
  external host effects. Existing resource classifications remain intact.
- Focused source/limit/lifetime/mapping tests pass. Full preflight, final
  review, CI evidence, and consumer rollout are not yet complete.

## Remaining acceptance work

Audit all execution paths, especially typed
hard-error latching, coroutines, source-map remapping through compiler
optimizations, and errors propagated from a different program. Add snapshots
for remaining resource-failure and host-panic fallbacks. Exact caller naming,
Unicode/control-character handling, repeated frame bounds, and reused source
names now have focused regression coverage.

Finish README/examples/current-state/progress updates, canonical release
preflight, required reviews/checks, and PR `ci-bench` evidence. Never run
performance benchmarks locally. Merge Rustia with a merge commit and
fast-forward the shared checkout before consumer PR merges.

Then migrate WOTS (both packages), civfun (required CIV issue and independent
correctness/quality reviews), and Grabby Next (draft claim before edits,
exact five-crate vendor refresh, all six locks/resolution/digest, required
reviews, console/scenario diagnostics, and `just merge`). Preserve each
repository's instructions and existing work. No deployment or standalone
Slowchop-console change is included.
