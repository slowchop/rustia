# Human-friendly script diagnostics

Status: runtime/API/CLI implementation merged as `759060b70c8e4f8c29a5bc48782d4e7b304bd204`
in [Rustia PR #32](https://github.com/slowchop/rustia-historic/pull/32). Its full release preflight,
correctness/conformance tests, platform and Node/V8 checks, and remote PR screening pass.
The controlled-client delivery record is below; this is not the entire public-release gate.

## Contract

The host report explains script failures using the exact executed source,
an honest expression span (or line-only fallback), and a bounded logical call
trail. Guest error values, message strings, protected-call behavior, fuel,
and location suppression remain language semantics, not presentation policy.
Rust backtraces remain conventional function/file/line reports.

`Error` privately owns a non-exhaustive `ErrorKind` and exposes `kind`, `into_kind`,
`as_vm_error`, `source_location`, `source_span`, `diagnostic`, and `report`.
`Error::from` accepts facade-owned failure payloads; lower-level engine
conversions remain private; `Error::embed` is the concise message constructor and
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

Facade runtime/source/compile errors and diagnostic data are independent public
types. `VmError::kind()` exposes a non-exhaustive category without raw heap IDs
or bytecode positions. Shared source snapshots preserve the existing retention
lease across facade wrapping, reset, and VM drop. A non-string guest failure
returned through a Rust callback is re-raised as its original formatted text;
this prevents stale/foreign guest-object IDs crossing VM lifetimes. Guest-only
protected calls retain their object identity. The external `facade_errors`
regressions cover both sides of this boundary.

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
- Source/limit/lifetime/mapping, typed hard-error and coroutine propagation,
  O2 remapping, foreign-program errors, resource-failure and host-panic reports
  are covered by the full passing workspace/conformance and release gates.

## Controlled-client delivery

- [WOTS #5](https://github.com/slowchop/wots/pull/5), merged: stock/cold-open source
  names, named custom packages and reports at terminal/save-restoration boundaries.
  Full workspace formatting, strict Clippy, tests and both playthroughs pass.
- [civfun #43 / CIV-75](https://github.com/civfun/civfun/pull/43), merged: actual map/include
  paths, typed runner context and I/O/SQL causes, and CLI reports. Includes remain uncached.
  All 53 mapgen-host tests, strict Clippy, CLI smoke, audit and both independent reviews pass.
- [Grabby #1025](https://github.com/slowchop/grabby-next/pull/1025):
  exact five-crate vendor source, canonical dependency closure and six root locks,
  rotated hardened identities, bounded ASCII console reports, generated-prefix mapping,
  exactly-once runtime failures and named scenario results. Guest leaf behavior is unchanged.
  The PR is the authoritative record for its full-suite, rendered, review and merge gates.

Every client uses the exact merged Rustia revision above. The shared checkout was fast-forwarded
before final client validation and merges. Existing dirty user checkouts were preserved.
No deployment, standalone Slowchop-console change, or local performance benchmark is included.
Remote screening success is not a claim of a measured diagnostic speedup or zero overhead.

## Deliberate boundaries

Keep extending oracle runtime-error-phase parity independently of presentation. This is not a debugger,
general transpiler source map or reconstruction of metadata lost through guest string rethrows.
Unicode/tab spans use an honest location-only fallback where a plain-text caret would be misleading.
Future public-release and real-host/browser goldens remain their own gates.

## Caller-selected and wrapped-coroutine reports (#18)

For positive `error` levels selecting a caller, report source, spans and wrapper
mapping come from that exact caller frame/program. Source names are labels and
may be reused by hot reload; matching a name is not proof of source identity.
When exact caller coordinates are unavailable, omit the excerpt rather than
quoting the throwing chunk. Source-retention budgets still permit location-only
reports, and hardened leaves retain their existing diagnostic restrictions.

`coroutine.wrap` retains the original handle-free diagnostic when propagating a
failure. Its guest error value/text and protected-call identity stay unchanged.
This metadata survives yield/resume and cross-chunk calls. Explicit guest string
rethrows after `coroutine.resume` still select the new `error` call's site; reports
do not parse source locations from strings or reconstruct discarded provenance.

Cross-profile regressions in `crates/rustia/tests/issue_18_19.rs` cover level-2/3
selection, reused names, mapped spans, generated caller failures, zero source
budget, retained reports after reset, wrapped coroutine suspension and manual
string rethrows. The `index_write_error_messages.luau` oracle fixture also pins
non-table write messages and wrapped-coroutine guest failure text.
