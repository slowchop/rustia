# Release compatibility and API audit

Status: in progress. Baseline: `07f3ee7ae30b5803251a3b1d736760361f52a8b7`.
This follows the private acceptance recorded in [93](93-private-api-acceptance.md).
It does not authorize publication or a version change.

## Delivery order

1. Audit language error behaviour, method lookup and coroutine boundaries against
   the pinned Luau 0.709 oracle. Fix confirmed gaps with conformance fixtures,
   four-profile regressions and bounded differential coverage. Reconcile older
   parity claims with executable evidence.
2. Obtain fresh CI benchmark evidence for the resulting candidate. Check the
   workload, revision and measurement provenance before updating comparisons.
   Local correctness runs provide no performance evidence.
3. Review the final API using Grabby, civfun and WOTS. Record concrete call sites,
   conversion/failure/lifetime contracts and any necessary migrations. Complete
   required checks and independent review before merging changes.

## Broader diagnostic audit

The reproducible [27-case inventory](release-error-audit.json) checks protected
runtime errors, missing/wrong builtin arguments, table mutation, coroutine
validation, arbitrary guest error values and frontend rejection. On `189ae963`,
14 cases match and 13 expose the differences below. This sample complements the
existing error-phase, builtin argument-order, coroutine and conformance tests;
it is not exhaustive language parity.

Build the locked fuzz `error-oracle-worker` with `--features oracle`, then run
`node tools/audit-release-errors.mjs`. The tool emits the complete observations
and exits nonzero when differences remain. It never rewrites expectations.
Only the oracle vehicle's source-name wrapper is normalized. No line number,
argument detail or message text is removed. Successful object addresses are
excluded by guest-side type labels.

| Case | Remaining difference |
|---|---|
| Length of nil | Generic type error rather than Luau's length error |
| Missing `rawget`, `rawset`, `type` arguments | Missing numbered argument diagnostics |
| Invalid `string.len`, missing `string.sub` argument | Missing function/argument/type details |
| Four-argument `table.insert`, invalid `table.sort` comparator | Wrong argument diagnostic |
| Invalid key to `next` | Extra source prefix |
| Invalid `coroutine.create` / `coroutine.resume` argument | Missing function/argument/type details |
| Freezing an already frozen or protected-metatable table | Wrong diagnostic text |

These are known gaps, not deliberate differences. Follow-up fixtures must test
direct and aliased calls, argument side effects and validation precedence.
Structured host errors and resource-exhaustion rules must remain intact. The
pending-free manifest describes its executable inventory; it does not erase
these independently observed findings.

The intentional differences remain the selected environment contracts:
reproducible iteration/RNG/numeric choices and deterministic capability limits,
as described in `05-luau-target.md`, `50-conformance.md` and `72-host-services.md`.
They must not be used to excuse ordinary language or builtin errors.

## Initial oracle findings

The existing `error-oracle-worker` was rebuilt from the baseline with its locked
`oracle` feature. Each source below was run in separate `luau` and `rustia`
processes. These are confirmed gaps, not accepted divergences.

### Method selection follows argument execution

```lua
local t = {m = function() return 'old' end}
local function arg()
    t.m = function() return 'new' end
    return 1
end
return t:m(arg())
```

Luau returns `new`; the baseline Rustia returns `old`. Computed receivers and
upvalues are captured before arguments, but a local receiver (including a
parenthesized local) is read after them, matching Luau `getExprLocalReg`. Method
selection observes argument effects. Cover fixed results,
return expansion, table append, nested argument expansion, varargs, argument
failure and argument suspension. Parenthesized/dot-call forms need separate
oracle expectations; they must not inherit colon-call ordering by assumption.

### Missing-method errors depend incorrectly on call shape

```lua
local t = {}
local function arg() observed = 1; return 1 end
local ok, e = pcall(function() return t:missing(arg()) end)
return tostring(ok)..':'..tostring(observed)..':'..tostring(e)
```

Both execute `arg`. Luau names the missing method (`attempt to call missing
method 'missing' of table`); Rustia reports `attempt to call a nil value`.
Compare message content separately from the oracle vehicle's chunk-name wrapper.
Do not normalize away meaningful error text in regression expectations.

### A forbidden yield during lookup has an extra error prefix

```lua
local t = setmetatable({}, {__index = function()
    coroutine.yield('lookup')
    error('failed')
end})
local c = coroutine.create(function() return t:missing() end)
local a, b = coroutine.resume(c)
local d, e = coroutine.resume(c)
return tostring(a)..':'..tostring(b)..':'..tostring(d)..':'..tostring(e)
```

Both reject the first resume and leave a dead coroutine. Luau returns the bare
`attempt to yield across metamethod/C-call boundary`; Rustia prepends its source
name and line. Replacing `t:missing()` with `t.missing` reproduces the same gap.
This is a rejected yield, not a successful suspended metamethod continuation.

The rejected-yield prefix fix now passes the four-profile facade regression for
ten contexts. Its new conformance fixture was independently run through the
pinned oracle (`ok / 0 / pass`). Full CI and independent review remain pending.

Method ordering now passes 14 oracle-derived call shapes, eight missing-method
error shapes and nine failure/GC/receiver/vararg cases across four profiles,
including double suspension and ordinary dot-call controls. Expanded method
lookups use validated continuation metadata; fixed calls select after arguments.
Missing-method messages now use checked lookup in these paths. Malformed metadata is rejected before argument execution. Explicit GC cases,
argument failure, protected-call recovery and repetition after reset pass.
The differential smoke passed before the local-receiver/append-vararg follow-up
and must be rerun for the final source. Compiler unit tests (124), VM unit tests (244), existing
builtin expansion/order tests, ten deferred-call tests and strict workspace
Clippy pass. The initial full corpus passed (725 fixtures / 519 refs); it predates the
local-receiver/append-vararg follow-up and is not final-candidate evidence.

## Completion evidence still required

- Conformance fixtures derived from the oracle and passing implementation tests.
- A broader error-phase/order audit beyond the three initial reproductions.
- Fresh remote benchmark results, with comparisons updated only where supported.
- Application-based API review and any resulting migrations.
- Required CI and independent review on the final changes.

## Delivery status

The method/vararg/yield batch is in [PR #70](https://github.com/slowchop/rustia/pull/70).
Its initial head passed eight target builds, native macOS arm64, Node/V8 and
differential CI. Preflight found stale generated publishability file counts;
those counts have been regenerated. A subsequent malformed-metadata regression
also made hardened-program validation reject out-of-range late receiver registers
at construction, matching runtime validation. Final-head checks remain required.

The method fixture now contains 31 cases, including a local receiver changed
after argument suspension, an upvalue control, and a parenthesized local. All
four profiles pass it before and after reset. The broader diagnostic findings
are tracked here because GitHub Issues is disabled for this repository.

Independent reviews of the compiler/VM changes and conformance evidence found
no blocking issue. Eight additional receiver/argument probes matched the pinned
oracle; both complete fixtures returned `ok / 0 / pass`, and the diagnostic audit
reproduced 14 matches and 13 differences. The review corrected an outdated case
count in the prose provenance record. These reviews do not replace final CI.

Preflight also exposed stale benchmark queue metadata and its rendered HTML.
Both were regenerated without changing measurements or speed claims. The
benchmark runner has reconnected and PR screening is running; the full fresh
cross-engine comparison remains pending.


## Dense-table regression investigation

All release checks passed on `897bfddb`. Its stronger
[CI acceptance comparison](https://github.com/slowchop/rustia/actions/runs/39694175435)
against `07f3ee7a` confirmed dense tables at +4.62% (99% interval +4.17% to
+5.04%), beyond the calibrated 2.08% screening threshold. Closures/upvalues
improved by 7.64%; the other three workload intervals include zero. A successful
workflow is not performance acceptance. Keep the compatibility batch unmerged
while investigating this result.

Static compilation found identical dense-workload bytecode and identical
normalized Linux LLVM IR for its sort and table-access helpers. The shared
interpreter gained register-slice selection and call arguments for deferred
method receivers. This is a code-generation hypothesis, not a measured cause.
The candidate stores the receiver binding in continuation state: a traced cell
for captured locals, or the stable value for uncaptured locals. Late read fuel,
collection and method lookup order remain unchanged. It removes the caller
register parameters from the continuation driver. Remote comparison is required
before claiming this candidate fixes the regression.

The application review found one concrete client correction: WOTS ignored its
per-feed fuel argument. [WOTS #9](https://github.com/slowchop/wots/pull/9), merged
as `7960b5af6621f8af49ca399da270a6463e014a57`, uses existing invocation limits;
zero keeps its established default, and explicit limits stay below the engine
ceiling. All 33 BBS tests, strict Clippy and independent review passed, including
exhaustion, preserved immediate writes and same-session recovery. No public
Rustia API addition or dependency change was needed. Grabby and civfun retain
the accepted application contracts recorded in document 93.


The [first isolation comparison](https://github.com/slowchop/rustia/actions/runs/39706980966)
measured `189ae963` against the baseline with four legs per side: dense tables
-0.59% (99% interval -1.21% to +1.45%), with no slowdown detected. The later
VM delta moves receiver bounds validation into `descriptor`; it does not add
work to dense-table guest execution. The receiver-binding refactor is withdrawn
in favor of a smaller isolation candidate that prevents expanded-call construction
(including descriptor setup and validation) from being inlined into shared
dispatch code. The descriptor helper was already out of line; its annotation
alone left dispatch unchanged in static inspection. All validation remains enabled.
Its performance effect remains unproven until remote measurement completes.
