# Error behaviour and fuzzing

Status: implementation in progress. The first six-target remote campaign is green;
expanded stateful coverage and the controlled-client rollout still need final acceptance.

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

Execution counts include rejected mutations, not distinct scripts or unique oracle
comparisons. This revision predates the expanded ten-operation stateful harness;
that expansion still requires a fresh remote campaign. Campaign artifacts retain
identity, toolchain, seeds, corpus and termination logs for 30 days.

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

- [ ] Audit boundary matrix against existing tests and oracle evidence.
- [x] Add focused phase, protected-call, coroutine, resource and recovery regressions.
- [x] Add bounded differential source harness with isolated pinned oracle.
- [x] Add bounded stateful embedding fuzz target and deterministic smoke corpus.
- [ ] Run remote 30-minute campaigns per target, then two-hour differential and stateful campaigns.
- [ ] Minimize confirmed findings; fix and permanently regress each finding.
- [ ] Run canonical release preflight and required CI; obtain PR ci-bench evidence separately.
- [ ] Merge Rustia and update WOTS, civfun and Grabby to the exact merged revision.
- [ ] Run client regression/full gates and required reviews; merge client PRs.
- [ ] Reconcile CURRENT, progress, fuzz documentation and campaign evidence.

Campaign artifacts must identify revision, toolchain, seed/corpus and termination
status. A timeout requires triage, not automatic classification as a language
mismatch. A clean bounded campaign is not proof that no bugs exist.

Out of scope: yieldable require, diagnostic redesign, GC tuning, new platform
coverage, public publishing and deployment. No local performance measurements.
