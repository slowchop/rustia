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
error shapes and six failure/GC/receiver/vararg cases across four profiles,
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
