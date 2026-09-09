# Error behaviour and fuzzing

Status: implementation in progress. No campaign or client rollout is yet complete.

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

## Delivery checklist

### Finding EF-1: quota-limited PUC short-string placement

The deterministic stateful sweep reduced a missing global to inserting the new
short key `counter` into a quota-limited PUC VM and immediately reading it.
Reset is not required. The quota candidate used the planned atom ID's fallback
hash before that atom had bytes, whereas subsequent reads used its string hash.
The candidate now uses a read-only pending-string hash overlay, including through
rehash, without interning anything before quota admission. The standalone
cross-profile regression passes with the fix. Broader quota/PUC validation and
remote campaign evidence remain pending.

- [ ] Audit boundary matrix against existing tests and oracle evidence.
- [ ] Add focused phase, protected-call, coroutine, resource and recovery regressions.
- [ ] Add bounded differential source harness with isolated pinned oracle.
- [ ] Add bounded stateful embedding fuzz target and deterministic smoke corpus.
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
