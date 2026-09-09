# Error behaviour and fuzzing

Status: complete. Runtime acceptance, both six-target remote campaigns, final CI
and the controlled-client rollout pass; all four implementation PRs are merged.

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

### Remote campaign evidence

[Run 34312173291](https://github.com/slowchop/rustia/actions/runs/34312173291)
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
[run 34334524153](https://github.com/slowchop/rustia/actions/runs/34334524153),
revision `347b0af7557535227d7b901713f0e3559c621b02`, with the same durations and
limits: lexer 27,116,785; parser 23,274,567; compiler 13,843,528; VM 462,119;
stateful 294,889; differential 2,045,334 executions. All six jobs succeeded,
without reported crashes, timeouts or mismatches. Subsequent changes only
strengthen integration-test assertions and documentation; production and fuzz
sources remain identical to that tested revision.

Execution counts include rejected mutations, not distinct scripts or unique oracle
comparisons. Campaign artifacts retain
identity, toolchain, seeds, corpus and termination logs for 30 days.

## Completed rollout

[Rustia #34](https://github.com/slowchop/rustia/pull/34) merged as
`aee98e7815460c2df014c9b20f92de8de5382814`. Canonical release preflight and final
PR checks passed, including remote benchmark screening; the
[merged-main CI](https://github.com/slowchop/rustia/actions/runs/34408292742)
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
