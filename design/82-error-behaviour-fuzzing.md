# Error behaviour and fuzzing

The original runtime acceptance, both six-target remote campaigns and the controlled-client
rollout passed. All four implementation PRs merged. Links to `rustia-historic` and
controlled-client repositories below require private access.

The [public launch smoke](https://github.com/slowchop/rustia/actions/runs/44695193576)
passed on `e54fd4e`. The public workflow runs the release-diagnostic audit and bounded
differential smoke on pull requests. Its six-target campaigns require manual dispatch.

## Contract and evidence

Language semantics follow the pinned Luau 0.709 oracle, or PUC 5.1.5 for that
compatibility profile. Host reports are presentation, never a replacement for
guest error values. Rust failures retain ordinary typed causes and backtraces.

| Boundary | Required observations |
| --- | --- |
| Source / compilation | Rejection precedes any guest side effect, including invalid uncalled functions |
| Ordinary execution | Protected calls preserve guest error values; earlier heap mutations survive failure |
| Protected calls / coroutines | Handler failure and resumed failure unwind correctly and close captured locals |
| Typed invocation | Staged records publish only on success; hard aborts cannot be hidden by protected calls |
| Host panic | Contained panic poisons execution until reset; aborting Rust builds cannot promise containment |
| Installation | Failed candidate restores guest state; external host/resolver effects remain outside rollback |
| Conversion | Returned-value conversion failure does not imply guest execution was rolled back |
| Reset / diagnostics | Old handles stay stale; retained reports remain valid even after VM destruction |
| Resources | Test ordinary and transactional catchability separately; do not infer it from error wording |

`crates/rustia/tests/error_recovery.rs` begins the cross-profile recovery matrix.
Existing typed-invocation, transactional-installation, module-resolution and
diagnostic tests remain part of acceptance; new cases must fill gaps rather than
replace them with weaker smoke checks.

### Boundary evidence map

- Source and compiler rejection: `invalid_uncalled_function_prevents_all_execution`
  and `compile_rejection_in_uncalled_function_precedes_side_effects` in `error_recovery.rs`.
- Ordinary mutations and conversion: `ordinary_failure_keeps_prior_mutations_and_vm_remains_usable`
  and `owned_conversion_failure_does_not_roll_back_guest_execution`.
- Protected values / captured locals: `protected_errors_preserve_object_identity_and_close_captured_locals`
  and `coroutine_failure_closes_upvalues_and_preserves_error_identity`.
- Ordinary memory exhaustion: `ordinary_quota_oom_is_catchable_and_reset_recovers`
  first asserts typed `OutOfMemory`, then tests catchability and post-reset reuse.
- Transactional memory/fuel: existing `typed_invocation.rs` tests
  `deterministic_oom_escapes_pcall_and_discards_the_outer_journal` and
  `fuel_is_shared_and_uncatchable_across_calls_in_one_invocation`.
  The first checks the caller lane remains empty; the second attempts `pcall`
  around each fuel-charged host call rather than relying on the test name.
- Record and yield boundaries: `record_limit_is_hard_even_if_callback_and_guest_try_to_swallow_it`,
  `resumed_protected_coroutine_cannot_swallow_a_hard_yield_boundary` and
  `resumed_xpcall_coroutine_cannot_swallow_a_hard_yield_boundary`.
- Poisoning: existing `host_panic_rolls_back_poisons_and_reset_recovers_bindings`
  and `host_service_panic_rolls_back_records_poisons_and_leaves_reset_usable`.
- Installation: existing `quota_failure_rolls_back_installation_and_keeps_the_vm_reusable`
  and `rollback_invalidates_preexisting_program_caches_before_handle_reuse`.
- Retention: `retained_reports_survive_reset_without_reviving_handles` and
  `builder_profile_globals_can_be_rebound_after_reset`.

The new recovery tests pass across all four profiles. This map identifies tests,
not a claim that the full acceptance suite or all optimizer paths have passed.
Optimized protected-call recovery is compared with default compilation across all
four profiles, repeated execution and reset. The expanded stateful target includes
callback failures, hard fuel aborts, conversion failures and cyclic live handles.
Client-specific regressions remain part of the rollout acceptance.

### OOM recovery after failed calls (#21)

VM error tracebacks are internal protected-call state, released when the public
execution entry returns to the host. Saved traceback and coroutine debug frames
retain the function/environment metadata required by debug APIs, but exclude
execution-only register and stackless roots. Collection retries can therefore
reclaim unreachable failed-call allocations before the next allocating call,
including after `pcall`, `xpcall` and a retained dead coroutine. This is collection,
not rollback: published guest writes and externally rooted values stay live.

`quota_oom_releases_failed_call_roots_before_the_next_allocating_call` repeats a
local-table allocation bomb and immediate allocating function calls under ordinary
and transactional execution on all four profiles, retaining the original error.
`quota_oom_chunk_recovery_preserves_published_values_and_captured_locals` covers
expanded arguments and preservation of globals, captured locals and external
roots. `protected_and_dead_coroutine_oom_release_local_allocations` covers ordinary
protected calls and retained coroutine handles. Existing hard-abort/journal tests
remain required. Host OOM reports include the stopped source line, with the same
bare Luau guest error value and unchanged hardened-leaf diagnostic policy.
`oom_host_reports_distinguish_generated_wrappers_from_user_expressions` checks
that generated prefix failures do not blame mapped user input and that user
expression reports preserve their mapped spans.

### Remote campaign evidence

[Run 34312173291](https://github.com/slowchop/rustia-historic/actions/runs/34312173291)
completed successfully on revision `0580e4bfb46fb867dd337b972cf426e9e3acd97d`.
Lexer, parser, compiler and VM ran for 30 minutes each; stateful and differential
each ran for 30 minutes plus a two-hour extension. Logs report no crashes,
timeouts or differential mismatches.

| Target | Fuzzer executions |
| --- | ---: |
| Lexer | 38,817,118 |
| Parser | 23,428,491 |
| Compiler | 14,578,675 |
| VM | 659,608 |
| Stateful | 466,534 |
| Differential | 1,919,974 |

The expanded ten-operation stateful harness also passed in
[run 34334524153](https://github.com/slowchop/rustia-historic/actions/runs/34334524153),
revision `347b0af7557535227d7b901713f0e3559c621b02`, with the same durations and
limits: lexer 27,116,785; parser 23,274,567; compiler 13,843,528; VM 462,119;
stateful 294,889; differential 2,045,334 executions. All six jobs succeeded,
without reported crashes, timeouts or mismatches. At that campaign closeout,
subsequent changes only strengthened integration-test assertions and
documentation; production and fuzz sources remained identical to that tested
revision. This dated result does not cover later harness expansions below.

Execution counts include rejected mutations, not distinct scripts or unique oracle
comparisons. Campaign artifacts retain
identity, toolchain, seeds, corpus and termination logs for 30 days.

## Completed rollout

[Rustia #34](https://github.com/slowchop/rustia-historic/pull/34) merged as
`aee98e7815460c2df014c9b20f92de8de5382814`. Canonical release preflight and final
PR checks passed, including remote benchmark screening; the
[merged-main CI](https://github.com/slowchop/rustia-historic/actions/runs/34408292742)
also passed before any consumer merged. This records correctness and delivery,
not a new performance claim or completion of the wider public-release gates.

All active consumers were verified against that exact Rustia revision:

- WOTS #6: full workspace formatting, strict Clippy, tests and stock/cold-open
  playthroughs passed.
- civfun #44: all 55 mapgen-host tests, strict Clippy, scoped formatting, seeded
  CLI generation and independent correctness/quality reviews passed.
- Grabby #1027: exact five-crate vendor identity, unchanged lock/resolution
  closure across six roots, full seven-crate tests, 81 scenario tests, four
  server Luau process tests, native/Wasm leaf identity, strict lint, independent
  reviews and required CI passed before the repository merge command.

Consumer PR references identify the controlled-client delivery; this public
record deliberately omits private issue and infrastructure details.

## Delivery checklist

### Finding EF-1: quota-limited PUC short-string placement

The deterministic stateful sweep reduced a missing global to inserting the new
short key `counter` into a quota-limited PUC VM and immediately reading it.
Reset is not required. The quota candidate used the planned atom ID's fallback
hash before that atom had bytes, whereas subsequent reads used its string hash.
The candidate now uses a read-only pending-string hash overlay, including through
rehash, without interning anything before quota admission. The standalone
cross-profile regression, PUC chain/rehash/rejected-write regressions and the first
remote campaign pass with the fix.

- [x] Audit boundary matrix against existing tests and oracle evidence.
- [x] Add focused phase, protected-call, coroutine, resource and recovery regressions.
- [x] Add bounded differential source harness with isolated pinned oracle.
- [x] Add bounded stateful embedding fuzz target and deterministic smoke corpus.
- [x] Run remote 30-minute campaigns per target, then two-hour differential and stateful campaigns.
- [x] Minimize confirmed findings; fix and permanently regress each finding.
- [x] Run canonical release preflight and required CI; obtain PR ci-bench evidence separately.
- [x] Merge Rustia and update WOTS, civfun and Grabby to the exact merged revision.
- [x] Run client regression/full gates and required reviews; merge client PRs.
- [x] Reconcile CURRENT, progress, fuzz documentation and campaign evidence.

Campaign artifacts must identify revision, toolchain, seed/corpus and termination
status. A timeout requires triage, not automatic classification as a language
mismatch. A clean bounded campaign is not proof that no bugs exist.

Out of scope: yieldable require, diagnostic redesign, GC tuning, new platform
coverage, public publishing and deployment. No local performance measurements.

## Focused builtin error-phase follow-up

The private milestone's later audit found valid builtin calls that specialized
compiler paths rejected before execution. For example, `'' .. tostring()` failed
during compilation, although Luau raises its missing-argument error at runtime;
placing that call in `if false` must leave the chunk executable. Equivalent tail,
local, and protected forms already used ordinary calls in several cases.

Specialization now declines unsupported argument shapes and unknown library
fields. Ordinary calls preserve evaluation and runtime failure behavior.
`pairs`/`ipairs` with extra arguments likewise use ordinary iterator calls.
Native handlers consume only the oracle-verified prefix below, after argument
expressions have executed. This is not a blanket change to builtin arity rules:
Luau still rejects `table.insert({}, 1, 4, 99)` during execution.

| Builtin | Arguments consumed; later arguments ignored |
|---|---|
| `tonumber`, `next` | First two |
| `table.concat` | First four |
| `table.getn`, `table.maxn` | First one |
| `table.create`, `table.remove` | First two |
| `table.find` | First three |
| `string.sub`, `string.byte` | First three |
| `string.len`, `string.upper`, `string.lower`, `string.reverse` | First one |
| `string.find` | First four |
| `string.split` | First two |

The capture used the existing locked worker with `mlua 0.11.6`, `mlua-sys 0.10.0`,
and `luau0-src 0.18.3+luau709`. Its archive matches the checksum in the lockfile
and the target pin; all 226 regular archive files matched the extracted sources.
The initial 97 bounded programs found 57 mismatches against the unchanged
facade candidate: 46 premature frontend rejections, seven failures of valid
runtime calls, and four differing protected-call results. Additional probes
verified the remaining consumed prefixes above and the rejected insert arity.
These are correctness observations, not performance evidence.

Capture each fixture by building the documented worker and feeding its source
to the pinned engine, for example:

```sh
cargo build --locked --manifest-path fuzz/Cargo.toml --features oracle --bin error-oracle-worker
fuzz/target/debug/error-oracle-worker luau < conformance/fixtures/regressions/builtin_ignored_extra_arguments.luau
```

| New fixture | Exact returned string from Luau 0.709 |
|---|---|
| `builtin_unreachable_calls.luau` | `ok` |
| `builtin_ignored_extra_arguments.luau` | `number;number;1;a;1;1;1;4;1;4;ab;3;97;ABC;abc;cba;1;a;number:123` |
| `iterator_ignored_extra_arguments.luau` | `ipairs=1:4;pairs=1:4;calls=2` |

`builtin_call_phases.rs` covers scalar, local, nested tail, protected, aliased,
dead-branch, and uncalled forms across all four profiles. It also asserts
preceding guest effects, ordered/exactly-once evaluation of ignored arguments,
and thrown extra arguments. Existing syntax/uncalled-`break` regressions retain
true frontend rejection. The differential generator now includes these call
shapes rather than only ordinary arithmetic/table failures. This follow-up merged in PR #64
after required CI and independent review; the evidence is recorded in
[93](93-private-api-acceptance.md). Yieldable `require` is also implemented under
module-resolution ABI v3. Broader language parity remains open.

### Expanded arguments and builtin lookup order

Independent review extended the same audit to effective argument counts.
An unparenthesized trailing call or `...` contributes all its results, including
zero. For example, `type(empty())` must fail for a missing argument, while
`type((empty()))` receives one nil and returns `"nil"`. Likewise, if `many()`
returns `4, 99`, `table.insert(values, 1, many())` supplies four arguments and
fails; `table.insert(values, 1, (many()))` supplies three and inserts `4`.
These were existing gaps in the initial facade candidate, not regressions from
the ignored-extra fixes above.

The compiler now uses syntax to preserve expansion across scalar, tuple,
return, table-constructor, and iterator contexts. It no longer assumes every
`math`/`string`/`table` call has one result. Parentheses retain their explicit
single-result meaning. Value and method calls share argument-window lowering,
including the implicit receiver, so a method wrapper preserves nested results.
The compiler-only conformance `concat` helper remains
known to return one value; ordinary guest builtins are not granted that shortcut.

Luau also distinguishes callee lookup order for recognized builtin calls.
`string.len('abc', extra())` resolves the recognized direct callee after its
arguments. If `extra()` replaces `string.len`, that replacement receives the
complete argument list. `(string.len)('abc', extra())` and a local function
alias capture their callee before the arguments. Recognition follows the exact
Luau 0.709 `Compiler/src/Builtins.cpp` list and syntactic global writes; a
library field assignment does not have the same effect as assigning the
library's global name. Unknown fields and non-recognized functions such as
`string.upper` retain ordinary order. Arguments that can execute guest code,
including metamethods, cannot use an opcode that bypasses the selected callee.

Expanded calls retain explicit, validated import metadata so lookup happens
after the trailing call completes. The continuation preserves this rule if
the argument suspends, and can suspend again in the selected callee. Metadata
is not a guest value, participates in program memory accounting, and is checked
before callbacks. Deferred callee slots have no placeholder load instruction;
the omitted import's fuel and logical steps are charged at the actual lookup.
These forms use scalar dispatch until an optimized executor
explicitly supports the contract; no wrapper closures or new opcodes are added.

The pinned captures add two runtime-error fixtures and one successful fixture:

| Fixture | Luau 0.709 observation |
|---|---|
| `builtin_expanded_insert_arity.luau` | Runtime error containing `arguments` |
| `builtin_zero_result_argument.luau` | Runtime error containing `argument` |
| `builtin_callee_evaluation_order.luau` | `103:3:3:103:103` |

`builtin_argument_expansion.rs` tests 21 oracle-derived cases across four
profiles. `builtin_callee_order.rs` covers direct/aliased/parenthesized and
replaced callees, source-later global writes, and metamethod effects. Twelve
additional oracle-derived cases suspend first in the argument and again in
the callee across fixed, return, append, nested, method and vararg forms, under a quota
and after reset. The bounded differential generator includes expansion and
lookup-order shapes. Required CI and independent review passed for the merged PR #64
follow-up, as recorded in [93](93-private-api-acceptance.md); this is a focused
compatibility fix rather than a claim of complete language parity.

### Nested expansion and recursive calls

Independent review found two remaining fusion-boundary gaps. The compiler could
still collapse the final call beyond the fixed two-, three-, or four-call
instruction shapes. For example, `{type(pass(empty()))}` incorrectly supplied
nil instead of zero arguments, and a deeper constructor silently dropped the
second result of `many()`. This also affected ordinary `select` calls; it was
not limited to builtin lookup order.

The call descriptor now records a variable-length sequence of argument windows,
optional late imports, and an optional innermost vararg tail. Calls beyond a
fixed instruction's shape use that explicit chain through the existing fixed,
return, and append terminals. Parentheses still stop expansion. The chain fits
the same checked register limit as other compiled calls, and its windows and
imports are validated before callbacks. The former fixed forms retain their
ordinary lowering when they can represent the whole expression.

The new continuation also exposed native stack growth in recursive expanded
arguments. Bytecode calls use an internal call request consumed by an iterative
driver. Saved callers retain the program, prototype, closure captures, environment,
thread environment, module cohort, registers, source/debug state and continuation;
return values and ordinary errors resume that complete state. Actual guest yields
carry the saved callers through the existing coroutine frame machinery. A base
case can yield, then resume and unwind through selected callees in different
functions or programs. Pending frames participate in the existing roots, program
retention and memory accounting, including explicit collection while callers are
pending. Recursion exhaustion remains a structured, uncatchable VM error.
Callable table/userdata targets in those chains resolve `__call` receivers
iteratively before dispatch, with the existing receiver order and recursion
charges. Their bytecode targets use the same saved frames; native and host
targets keep ordinary dispatch. Pinned controls cover recursive callable tables,
their suspension and guest error identity, and native receiver arguments.
Ordinary bytecode fallbacks use that driver too: a function resumed after
evaluating an expanded argument can make a later fixed, variable-result, vararg,
constructor or prefixed-return call without nesting Rust interpreter frames.
Existing eligible leaf and self-call shortcuts retain their own paths. Legacy
positive-aux expanded forms normalize into the shared continuation state.
Fallible program retention keeps pending targets, arguments, handlers and
completion frames in temporary VM root scopes. Raw protected-call regressions
force collection during retention and check that both scopes are restored after
an out-of-memory failure.

The review also found that the configured `RecursionLimit` had incorrectly
remained catchable in protected calls. It now follows the documented hard-abort
policy, including the VM's synthetic protected-call shortcuts, and aborts a
typed invocation before publishing staged records. The opt-in Luau conformance
call limit is separate: it reports the oracle's catchable `stack overflow`
runtime error. Ordinary recovery and typed rollback tests cover this distinction.

`builtin_nested_expansion.rs` captures 90 pinned-Luau programs: depths through
16, zero and multiple results, parentheses, constructors, method receivers,
varargs, repeated suspension, and recursive depth 20/100 including a suspended
base case. Two additional corpus fixtures record `2:3:false:false:2` for nested
result counts and `3:3:3:20` for self, mutual, callable and later ordinary recursive
calls. Four-profile ordering
controls also pin the shared Luau call semantics; `Puc51Compat` continues to select its
documented table/sort/format behavior rather than a second language compiler.
The combined typed-object candidate also keeps a fresh object alive in an
expanded-call continuation across both suspensions, repeated collection pressure,
and reset; its method remains callable after each resume.

Additional regressions cover mutual depth 20/100, suspended bases, original guest
error identity, protected saved-caller unwinding, configured exhaustion, and calls
between independently loaded programs with distinct captures, `setfenv`
environments and module cohorts across suspension. These address the
mutual-recursion native-stack abort found during independent review.

### Overlapping builtin resource limits

`string.rep` variable-work fuel is charged before the output allocation,
including its specialized opcode. This closes a gap where unary-result lowering
could omit the output-byte charge while an ordinary call charged it. A request
that exceeds both budgets reports fuel exhaustion first. Resource tests must
exercise each budget separately: `string.rep('x', 600000)` with a 512 KiB heap
and 2,048 fuel fails with `FuelExhausted`; with 1,000,000 fuel it reaches the
allocation check and fails with `OutOfMemory`. The public regression
`builtin_output_work_checks_fuel_before_allocation_across_call_shapes` checks
direct, aliased and parenthesized calls in all four profiles, plus reset/recovery.
These are embedding-policy tests, not standalone Luau oracle expectations.

## Retained exports and functions (2026-10-30)

The stateful driver now covers the existing [replacement contract](75-embedded-api.md#4-compiled-module-and-transactional-replacement-api)
with retained `Exports` and `Function` handles. It adds seven operations to the
existing eighteen: install a captured version, reject a mutating candidate,
detach a function, drop both export slots, clone exports, force collection, and
drop the detached function. After every operation an independent fixed host
array checks each retained version and mutable captured value.

The failed-install operation checks the changed value inside the candidate
before its deliberate runtime error. The post-operation check then proves the
change was rolled back in both its scalar upvalue and exported state table.
Collection first proves a weak witness is stored, removes its strong root, then
allocates eight discarded 256 KiB buffers. Their total plus the VM baseline
exceeds the 2 MiB guest quota, forcing collection even in `DeterministicSim`;
the missing weak witness proves collection happened. A detached function is
checked afterward while both export slots and the temporary guest alias are
absent.

Reset checks `Exports`, `Function`, live `Value`, `Table` and Rust-object handles
against exact retained/current decision epochs. At most one stale export and
one stale function survive to later operations, so fresh allocations and new
installations cannot silently revive them. Persistent live handles are bounded
to current/previous exports and one detached function. The 128-byte input,
4,096 fuel per execution and 2 MiB guest quota remain the driver limits.

The two added smoke tests execute 296 traces across all four profiles: nine
explicit sequences per profile, one maximum-length collection trace per
profile, and 64 seeded 32-byte traces per profile. They mix retained handles
with the existing failure, object, yielded-module, close and reset operations.
The complete regular fuzz-workspace tests and strict Clippy validate the change;
no production runtime/API behavior or conformance inventory changes.

```sh
cargo test --locked --manifest-path fuzz/Cargo.toml
cargo clippy --locked --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings
```

Independent review caught and corrected two vacuous-proof risks: an unchecked
in-candidate mutation and an unchecked initial weak insertion. The resulting
source/tests have no remaining review findings. This records local correctness
coverage. No new coverage-guided remote campaign, benchmark measurement,
publication or supported-release acceptance is claimed.

### Targeted type-grammar differential coverage

The generated differential target includes twenty-one type shapes in addition to its
40 existing runtime shapes. These cover valid/invalid assertions, alias defaults,
pack defaults, generic function signatures, ordering and result adjustment. CI smoke takes its shape bound
from the generator and exercises all eight type variants. Parser and facade
negative tests additionally require Source rejection before any guest write in
all four profiles. The existing `export_type_assertions.luau` pinned fixture now
also exercises erased defaults and valid pack/grouping boundaries.

This closes two concrete coverage holes, not the full type-grammar gate. The
table/return parser now validates the pinned focused forms; broader syntax parity remains open. Raw frontend mutation fuzzing checks robust
handling, not independent syntactic acceptance; the oracle generator compares
only the families it generates. Independent-frontend grammar comparison remains
planned. No new remote fuzz campaign or performance claim is implied.

Generic function type coverage also ports source excerpts from pinned Luau 0.709
`tests/TypeInfer.generics.test.cpp` and `tests/Parser.test.cpp`, with named upstream
tests, exact commit provenance and MIT attribution in `generic_function_types.luau`.
Scopes and added runtime checks make the runnable excerpts independent; static
inference expectations are not ported. Malformed generic/named/pack forms are
checked for rejection before guest effects on all four profiles. The differential
smoke includes four additional valid/invalid generic-function shapes.

Return-annotation and table-type coverage ports additional pinned `Parser.test.cpp`
excerpts with named upstream tests and MIT attribution in `return_type_annotations.luau`
and `table_type_grammar.luau`. Four additional generated shapes exercise eight variants
per family, including malformed forms rejected before guest effects. These targeted
families do not establish the whole semantic/platform acceptance tracked in issue #31.

The compositional family crosses eight type leaves (including malformed forms),
eight grouping/table/indexer/function wrappers and four alias/return/assertion/default
contexts. The existing CI differential smoke executes all 256 combinations against
the locked oracle in addition to the fixed families and seeded random inputs.

Two further typeof families cover ordinary/interpolated/function expressions and
malformed expression syntax. Copied pinned source and no-execution controls are
in `typeof_expressions.luau`; facade rejection tests require no guest writes on
any profile. This closes the confirmed expression-skipping gap, not all of #31.

Erased typeof function bodies enforce loop and vararg context while signatures
retain the enclosing context. Ordinary invalid function bodies retain their
existing Compile error phase. A common 512-frame parser budget spans statement,
expression, type and function-signature boundaries; deep-cycle unit, facade and
bounded frontend smoke regressions require structured errors instead of stack
exhaustion. Two additional differential families exercise these context rules.

The `type_grammar` facade regressions are enabled for native and Wasm test runners.
Node/V8, headless WebKit and the explicit real-browser script run them in both
feature modes, alongside the existing shared contracts. The browser-runner
self-test rejects a failure isolated to this suite and verifies eighteen executed
suite groups; it does not substitute for actual browser execution.
