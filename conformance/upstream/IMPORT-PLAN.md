# Luau 0.709 Conformance Import Plan

This plan prepares the full Luau implementation work without depending on CI.

## Oracle

- Upstream repository: <https://github.com/luau-lang/luau>
- Commit: `b968ef742741bb2b703afc3b3c53f06608c87481`
- License: MIT, from upstream `LICENSE.txt`
- Primary upstream source: `tests/conformance/*.luau`
- Upstream harness: `tests/Conformance.test.cpp`, using default compile options
  `optimizationLevel = 1`, `debugLevel = 1`, `typeInfoLevel = 1`, standard libs
  opened, `print` silenced unless verbose, `loadstring` and `collectgarbage`
  registered for test compatibility, sandboxed globals, and `_G` bound to the
  current global environment.

## Import Shape

Port whole upstream scripts first, rather than splitting every `assert` into a
separate fixture. They are already self-checking and preserve upstream line
numbers/error expectations. Smaller rustia-only fixtures remain useful for
focused P1 smoke coverage.

Required conformance harness additions before vendoring:

- `expect=pass` for whole scripts that should execute without error.
- `expect=fail:<needle>` for whole scripts that intentionally fail.
- `status=pending-impl` or equivalent skip accounting, so the complete upstream
  inventory can be tracked without making the suite red before each feature is
  implemented.
- A test harness profile that installs conformance-only helpers (`assert`,
  `print`, `loadstring`, optional `collectgarbage` behavior) without exposing
  them as public embedding API.
- Per-fixture provenance metadata: upstream path, commit, license, local path,
  oracle command, and implementation stage.

## Upstream Files

The pinned upstream conformance corpus contains these scripts:

```text
apicalls.luau
assert.luau
attrib.luau
basic.luau
bitwise.luau
buffers.luau
calls.luau
clear.luau
closure.luau
constructs.luau
coroutine.luau
coverage.luau
cyield.luau
datetime.luau
debug.luau
debugger.luau
errors.luau
events.luau
exceptions.luau
explicit_type_instantiations.luau
gc.luau
ifelseexpr.luau
interrupt.luau
iter.luau
literals.luau
locals.luau
math.luau
move.luau
native.luau
native_integer_spills.luau
native_types.luau
native_userdata.luau
ndebug_upvalues.luau
pcall.luau
pm.luau
safeenv.luau
sort.luau
strconv.luau
stringinterp.luau
strings.luau
tables.luau
tmerror.luau
tpack.luau
types.luau
userdata.luau
utf8.luau
vararg.luau
vector.luau
vector_library.luau
```

## Implementation Staging

Stage A, core language bootstrap:

- `assert.luau`
- `basic.luau`
- `literals.luau`
- `locals.luau`
- `constructs.luau`
- `calls.luau`
- `vararg.luau`
- `closure.luau`
- `ifelseexpr.luau`
- `move.luau`
- `clear.luau`

Stage B, tables, metatables, errors, iteration:

- `tables.luau`
- `events.luau`
- `tmerror.luau`
- `iter.luau`
- `sort.luau`
- `pcall.luau`
- `errors.luau`
- `exceptions.luau`
- `safeenv.luau`

Stage C, stdlib/runtime surface:

- `math.luau`
- `strings.luau`
- `stringinterp.luau`
- `pm.luau`
- `utf8.luau`
- `strconv.luau`
- `bitwise.luau`
- `tpack.luau`
- `buffers.luau`
- `coroutine.luau`
- `gc.luau`

Stage D, public parity extensions:

- `vector.luau`
- `vector_library.luau`
- `userdata.luau`
- `datetime.luau`
- `debug.luau`
- `coverage.luau`
- `types.luau`
- `explicit_type_instantiations.luau`

Stage E, Luau C API/native-codegen specific tests:

- `apicalls.luau`
- `cyield.luau`
- `debugger.luau`
- `interrupt.luau`
- `native.luau`
- `native_integer_spills.luau`
- `native_types.luau`
- `native_userdata.luau`
- `ndebug_upvalues.luau`

Stage E is tracked for public parity, but many rows are not first-order P1
interpreter requirements because they validate the Luau C API, native codegen,
or debugger hooks rather than ordinary script execution.
