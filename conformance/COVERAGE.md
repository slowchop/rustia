# Conformance Coverage Notes

This file tracks the shape of the coverage work. The executable source of truth is `MANIFEST.tsv`.
The declared behavior-topic catalog is `REFS.tsv`.

The [parser inventory](upstream/luau-0.709/README.md#parser-tests) and [lexer inventory](upstream/luau-0.709/README.md#lexer-tests) map the pinned upstream frontend tests. The runtime fixture totals below come from the manifest.

## Current Status

- The current manifest contains 736 fixtures and 526 refs: 718 executable `luau-0.709` fixtures, 0
  `pending-oracle` P1 smoke fixtures, 18 `puc-lua-5.1.5` PUC 5.1.5 compat fixtures, and 0 whole upstream
  Luau 0.709 conformance scripts as `pending-impl` inventory.
- The `puc-lua-5.1.5` rows are the `Puc51Compat` differential corpus: expectations were captured offline
  from the pinned PUC Lua 5.1.5 oracle (`third_party/lua-src` submodule, regeneration procedure in
  `conformance/README.md`), covering PUC table `next()` enumeration order under mixed/scalar, string-heavy,
  boolean, negative/fractional, and sparse jumping-k keys, deletion-during-`next()`, tombstone
  resurrection, `#` border evolution across rehash boundaries, constructor pre-sizing, `table.insert` /
  `table.remove` border behavior, `%.14g` number formatting through `tostring`/concat/`table.concat` /
  `string.format("%s")`, and PUC `auxsort` tie order.
- No manifest rows or catalog refs remain pending. P1 smoke fixtures are now pinned to Luau 0.709 oracle
  evidence, and the whole upstream `apicalls.luau`, `assert.luau`, `attrib.luau`, `basic.luau`, `calls.luau`, `constructs.luau`,
  `ifelseexpr.luau`, `clear.luau`, `closure.luau`, `coroutine.luau`, `coverage.luau`, `cyield.luau`, `move.luau`, `bitwise.luau`, `interrupt.luau`, `iter.luau`, `literals.luau`, `locals.luau`,
  `buffers.luau`, `datetime.luau`, `debug.luau`, `debugger.luau`, `errors.luau`, `events.luau`, `math.luau`, `native.luau`, `native_integer_spills.luau`, `native_types.luau`, `native_userdata.luau`, `ndebug_upvalues.luau`, `pm.luau`, `safeenv.luau`, `sort.luau`,
  `exceptions.luau`, `explicit_type_instantiations.luau`, `gc.luau`, `pcall.luau`, `strconv.luau`, `stringinterp.luau`, `strings.luau`, `tables.luau`, `tmerror.luau`, `tpack.luau`, `types.luau`, `userdata.luau`, `utf8.luau`, `vararg.luau`, `vector.luau`, and `vector_library.luau` scripts are now executable.
- Executable upstream-derived fixtures currently cover assert basics and message conversion, numeric equality/comparison, unary `not`
  and numeric minus,
  logical `and`/`or`, conformance-silenced `print`, first-class no-op `print` global typing, single and multi-name locals, global name and `_G`
  environment reads/writes, local assignment/swap, anonymous-call local/global/table assignment target conflict, basic
  `if`/`elseif`/`else` truthiness and fallthrough, Luau if-expressions including `elseif` chains,
  nested conditions, and conditional branch evaluation, `while`, `repeat`/`until`, `do` blocks, `break`,
  `continue` in numeric `for`/`while`/`repeat` loops including repeat-condition closure capture,
  numeric `for`, local compound `+=`, explicit type-instantiation call suffixes, typed return packs,
  erased function type packs, erased native type-name annotations from `native_types.luau`, and contextual `type` keyword use as a local/parameter/generic-for binding name,
  string-literal, long-bracket string/comment syntax, and
  table-constructor call syntax, table field/index compound assignment, table-target conflict assignment, generic `for` over
  `ipairs`/`pairs`/manual `next` for array tables and hash-only tables, direct table-constructor iteration, direct single-result `next`, extra generic-for variables binding nil, array table literals, sequential and nil-only array
  table length/index reads, named/computed table fields, duplicate table field assignment order, array-key
  constructor stores, table index assignment, absent/non-integer/string/NaN key reads, nil/NaN key write
  errors, upstream `concat` helper rows, first-class stdlib function field reads/calls, scalar `tostring`, `type`/`typeof`, raw `rawequal`,
  `rawget`/`rawset`/`rawlen`, string length including embedded NUL decimal
  escapes, direct `string.find` first-result searches including anchored control-byte class patterns, direct `string.len`, direct single-result
  `string.byte`, direct ASCII `string.upper`/`string.lower`, direct `string.rep`, direct `string.gmatch`
  iterator closures over plain byte matches, line scans, ASCII word runs, and UTF-8 charpattern slices,
  direct `string.split`,
  direct `string.reverse`, direct `string.sub`
  clamping/negative-index ranges, direct `string.format` for `%s`, `%%`, `%q`, `%c`, integer, and
  fixed/exponent/general floating directives with basic width/precision handling, string method indexing for implemented string library functions,
  `table.concat` including start/end ranges and numeric hash-key ranges, basic
  `tonumber` including decimal, infinity, NaN payload, hex integer and hex-float string parsing,
  runtime rejection of source-literal-only binary prefixes and underscore separators, explicit-base
  integer parsing, base-16 `0x` prefixes, and unsigned explicit-base sign handling, direct `math.abs`,
  and direct scalar math helpers.
- Numeric operator coverage now includes upstream-derived `+`, `-`, `*`, `/`, `%` with both local update and
  runtime-visible RHS, `//`, finite `^`, NaN, infinities, sign-of-zero cases, unary numeric minus, and
  string-to-number arithmetic coercion. Numeric literal coverage now includes source-level binary literals
  via a reduced upstream `buffers.luau` fixture and underscore digit separators via the whole upstream
  `bitwise.luau` script.
- String operator coverage now includes upstream-derived string concatenation, concat type errors, string byte length with exact
  byte expectations including embedded NUL bytes from decimal escapes, and bytewise lexicographic relational
  comparison.
- Stdlib string coverage now includes direct first-result `string.find` calls for plain byte search, init
  normalization, numeric-string and fractional init coercion, anchored-pattern init misses, empty patterns, embedded NUL misses, dot-wildcard pattern cases, assignment expansion of
  start/end results, percent-escaped literal
  pattern bytes, anchored escaped-literal dot-star chunk-prefix patterns, non-capturing byte-oriented Luau
  patterns with classes and repetition suffixes, and anchored control-byte class
  patterns; direct `string.len` calls
  over empty strings, embedded NUL bytes, digit strings, and numeric coercion; direct single-result
  `string.byte` calls over ASCII, embedded NUL/newline bytes, negative indices, explicit ranges,
  multi-result byte ranges including clamped explicit starts, and out-of-range nil results; decimal and hexadecimal high-byte string escapes with exact byte preservation;
  direct ASCII `string.upper` and `string.lower` with embedded NUL preservation;
  direct `string.rep` with zero count, UTF-8/NUL preservation, empty-input large counts, and runtime loop
  counts; direct `string.split` with empty, present, absent, and trailing literal separators; direct
  `string.reverse` with empty strings and embedded NUL/control-byte preservation; direct `string.sub`
  calls with omitted end, empty ranges, clamped positive ranges, and negative indices; direct `string.match`
  for plain, simple dot-pattern, anchored greedy dot-star first-result cases, captures, nested captures,
  position captures, numeric backreferences, and decimal line-number captures used by upstream error stack
  parsing; direct `string.gsub`
  plain byte-pattern replacement with replacement counts, function and table replacements over whole-match
  and single-capture patterns, anchored optional-capture insertions used by upstream pattern helpers, the
  upstream `constructs.luau` `%s+` whitespace rewrite pattern, the upstream `errors.luau` `(%p)`
  punctuation-capture escaping pattern, byte-class ranges and percent classes used by upstream pattern-match
  set scans, dot-as-any-byte replacement scans, empty-pattern and anchor insertions, generic captures with
  numbered replacements, position captures, balanced `%bxy` spans, malformed-capture errors, byte-class ranges
  used by upstream UTF-8 continuation stripping, and no-capture `%1` replacement expansion; direct `string.char`
  byte construction, 256-argument construction via recursive multi-return expansion, empty result, range errors, and `string.byte` result
  expansion; plus direct
  `string.format` calls for string substitution, literal percent emission, `%*` tostring formatting,
  signed/unsigned integer base formatting, quote escaping, byte-character output, fixed/exponent floating-point output, zero-padding, left alignment, and string precision; and `string.pack`, `string.unpack`, and
  `string.packsize` coverage for Luau integer sizes, endian selection, explicit alignment, fixed strings,
  direct calls, zero-start unpack positions, large fixed-string packsize accounting, zero-terminated strings,
  sized strings, next-position returns, and representative error substrings. UTF-8 string-library coverage
  now includes the `utf8.charpattern` constant, anchored `string.find` validation against that pattern,
  `string.gmatch` iteration over that pattern with optional
  one-based byte-position captures, and direct `utf8.char`, `utf8.codepoint`, `utf8.codes`, `utf8.len`,
  and `utf8.offset` calls including ignored extra arguments, invalid-byte, and out-of-range error paths. A narrow `tables.luau`
  cleanup case covers the anchored uppercase-or-underscore class pattern used by upstream global cleanup.
  The whole upstream `stringinterp.luau` and `strings.luau` corpora are now executable.
- Table/iteration coverage now includes array literal construction, sequential array length/index reads,
  named and computed table fields, duplicate field overwrite/removal order, absent key reads, non-integer
  numeric keys, string-key bracket indexing, table index assignment, negative-zero numeric key normalization,
  table field/index compound assignment,
  nil/NaN key write errors, NaN-key reads returning nil, direct table-constructor traversal,
  `ipairs` stopping at the first raw nil array slot while ignoring `__index`/`__len`,
  Luau 0.709 `newproxy` nil/boolean-only argument semantics, numeric-string coercion across the arithmetic
  operator family including unary minus,
  `pairs` continuing past nil holes, Luau-style array-boundary length search for nil-tailed pre-sized arrays
  and numeric hash keys beyond a filled array prefix, callable `ipairs`/`pairs` iterator triples,
  `pairs` continuation after deleting the current hash key, `pairs`/manual `next, table`
  iteration over hash-only tables, weak-key/weak-value table cleanup after explicit and automatic collection, and combined
  weak key/value root liveness. The whole upstream `gc.luau` inventory row is executable under the conformance
  harness; its `setblockallocations` helper is currently a no-op shim, so the three allocation-blocking OOM
  robustness probes are execution coverage rather than allocator-failure equivalence coverage.
- Conformance harness API coverage now includes whole upstream `coverage.luau` through a gated `getcoverage`
  helper that records bytecode prototype line counters and metadata only for the conformance runner.
- Whole upstream `ndebug_upvalues.luau` is executable with a gated top-level `coroutine.yield` passthrough
  for the C-harness upvalue-inspection checkpoint; public default top-level yield behavior remains an error.
- Whole upstream `debug.luau` is executable, including thread-form `debug.traceback`/`debug.info` over
  coroutine yield/error snapshots, active-line reporting for multi-result native calls, function metadata,
  and arity reporting.
- Whole upstream `userdata.luau` is executable through a gated conformance host `int64` userdata payload with
  equality, order, arithmetic, `__tostring`, and mutable `.value` metamethod behavior.
- Whole upstream `native_userdata.luau` is executable through gated conformance host `vec2` and `vertex`
  userdata payloads, covering native-style field/method indexing and vector/vec2 composition.
- Whole upstream `native_types.luau` is executable through gated conformance native-type guard emulation:
  typed parameter annotations are compiled into runtime metadata and checked at function entry, with a scoped
  `is_native` helper for this oracle row.
- Upstream helper coverage now includes the `basic.luau` `concat(...)` helper for fixtures that need comma-joined
  `tostring` output including nil values. `tostring` coverage now includes Luau-shaped table/function
  identity strings, `__tostring` metamethod dispatch for tables and newproxy userdata, table and newproxy
  userdata `__index`/`__newindex` dispatch including key-validation and readonly absent-key cases, `__call`
  dispatch, table `__len` dispatch, numeric arithmetic, string concatenation, relational comparison, and
  equality metamethod dispatch, protected
  metatable visibility and setmetatable rejection, `loadstring`
  literal and dynamic-source chunk compilation in conformance mode including cross-program calls back into root
  functions, invalid literal source returning nil plus an error message, and invalid dynamic source returning
  chunk-name-prefixed error messages, dynamic-source rejection of bare/repeated semicolon empty
  statements and newline-separated parenthesized calls, and acceptance of string and table
  call arguments across a line break,
  `pcall` string-error source prefix objects including optimized `assert` failures, VM-raised arithmetic and
  numeric-for/function-target line attribution errors, exact arithmetic/comparison and numeric-for type-error text,
  missing-method/index diagnostics for colon calls, catchable recursive stack-overflow errors, `debug.traceback`
  as an xpcall handler and over named bytecode frames with level filtering, `debug.info` argument errors,
  active stack metadata, and native/bytecode function metadata, level-0 no-prefix errors, positive `error` stack-level
  line attribution, and missing-argument errors, including zero-result call
  expansion, in addition to scalar values. Base stdlib coverage now includes the upstream-facing
  `collectgarbage()`/`collectgarbage("collect")` no-op collection path, `collectgarbage("count")`,
  `collectgarbage("stop")`/`collectgarbage("restart")`, and `collectgarbage("step", size)`
  with numeric/boolean harness results, and compatibility `gcinfo()` numeric count results that can drop for
  legacy GC polling loops, plus native-function `getfenv`/`setfenv` behavior and `setfenv`
  for function, the currently running function frame, active stack-level, zero-result level-0 calls,
  and loadstring thread environments.
- Stdlib coroutine coverage now executes the whole upstream `coroutine.luau` script: resumable
  `coroutine.create`/`resume`/`wrap`/`yield`, resume-argument delivery back into yielded call sites,
  yielded result delivery to callers, running/normal/suspended/dead status, `coroutine.running`,
  `coroutine.isyieldable` across yieldable and non-yieldable native callback boundaries, immediate
  catchable yield rejection inside `tostring`, function-based `string.gsub`, `table.foreach`, and
  `table.foreachi` callbacks through both `pcall` and `xpcall`, independently yieldable child coroutines
  resumed inside those callbacks, weak-table
  collection of unreachable coroutine wrappers, `coroutine.close`, and the upstream xpcall continuation
  cases covered by `coroutine.luau`.
- Stdlib table coverage now includes `table.concat` over sequential array table literals with numeric/string
  elements, explicit string separators, start/end range arguments, and explicit numeric hash-key ranges;
  `table.getn` over literal and assigned array entries; `table.maxn` over empty tables, ignored negative
  and string keys, fractional numeric keys, large numeric hash keys, and nil removal; `table.create` over
  nil-filled pre-sized arrays, later in-capacity assignment, optional fill values, zero size, fractional-size
  integer conversion, numeric-string size coercion, negative-size errors, and table-overflow errors; plus
  `table.find` over array entries, optional start indices including nil default and zero-init errors, misses,
  booleans, first-nil stop behavior, numeric hash keys, and `__eq` metamethod dispatch; `table.insert` append/positioned-shift behavior
  including pre-sized nil arrays, hash-boundary append cases, explicit zero/sparse/negative positions,
  and non-error handling for the upstream NaN-position probe; and
  `table.remove` default, explicit-nil, positional, hash-boundary default, exhausted, and empty-table cases,
  `table.pack` positional construction plus `.n` argument-count preservation through nil holes, and
  `table.unpack` variable-result ranges including nil holes, empty ranges, pcall propagation, table-constructor
  expansion, nested select/unpack constructor expansion, and the Luau 0.709 7,999/8,000 large-result boundary; `table.clear`
  array/hash removal, table-argument type errors, readonly errors, and post-clear array reuse; and `table.move` copies including
  same-table overlap direction, optional destination return, empty ranges, nil-clearing, boundary-length
  maintenance, table-argument type errors, too-many/wrap-around errors, and readonly destination errors; `table.freeze`/`table.isfrozen`
  shallow readonly behavior including identity return, already-frozen errors, assignment/rawset rejection, and
  readonly `table.insert`/`table.remove` errors; legacy `table.foreach`/`table.foreachi` callback iteration;
  `table.clone` shallow-copy behavior including metatable preservation, thawed clone state, protected-metatable
  rejection, and table-argument errors; protected-metatable `table.freeze` rejection; `table.insert` zero-result arity when observed through
  `select("#", ...)`; and `table.sort` default number/string sorting, default `__lt` metamethod sorting, plus optional
  comparator calls, including extra argument ignoring, non-function comparator rejection, and readonly table errors.
- Raw stdlib coverage now includes `rawequal`, `rawget`, `rawset`, and `rawlen`, including extra argument
  evaluation/ignoring, raw table/string lengths, raw table writes, raw table reads, and nil/NaN key write
  errors for nil and non-nil assigned values, including the direct `calls.luau` incorrect-argument rawget/rawset rows.
- Stdlib math coverage now includes direct `math.floor`, `math.ceil`, `math.min`, `math.max`, `math.sign`,
  `math.clamp`, `math.round`, `math.fmod`, `math.pow`, `math.sqrt`, `math.deg`, `math.rad`, `math.pi`,
  `math.isnan`, `math.isinf`, `math.isfinite`, `math.sin`, `math.cos`, `math.tan`, `math.asin`,
  `math.acos`, `math.atan`, `math.atan2`, `math.sinh`, `math.cosh`, `math.tanh`, `math.exp`, `math.log`,
  `math.log10`, `math.ldexp`, `math.frexp`, `math.modf`, `math.random`, `math.randomseed`, `math.noise`, `math.map`, and `math.lerp` calls, including numeric string coercion, seeded PCG32 sequence checks, order-dependent NaN
  behavior for min/max, infinities for sign/round/predicates/log, clamped ranges, C-style remainders, finite
  powers/square roots, degree/radian conversion, zero-point libm transcendentals, log base dispatch, binary
  exponent scaling, unary-call extra argument discard,
  extrapolating remaps, and lerp endpoint behavior.
- Stdlib vector coverage now includes the whole upstream `vector.luau` and `vector_library.luau`
  scripts: vector creation/callability, component access, `type`/`tostring`, equality/`rawequal`,
  table-key hashing including `-0` and NaN-key rejection, vector arithmetic, `Dot`/`Cross` methods,
  and the `vector.*` math library surface.
- Stdlib `bit32` coverage now includes uint32 argument reduction including large exactly representable
  operands, numeric-string and fractional integer coercion, `band`/`bor`/`bxor` folds including zero-argument
  identities, `btest`, `bnot`, logical and arithmetic shifts including negative displacements and shifts beyond
  31, rotates modulo 32, `extract`/`replace` field operations, extra-argument ignore behavior, and bounds
  errors, `countlz`, `countrz`, and `byteswap`.
- Function coverage now includes local function declarations, anonymous function expressions, scalar calls,
  self-recursion by local function name, fixed parameter nil-fill/extra argument discard, `type(function)`,
  colon method-call expressions/statements with implicit `self`, table-field function declaration assignment,
  colon method declarations with implicit `self`, method declarations against global/table chains,
  chained method calls returning `self`, nested table method assignment, higher-order fixed-point
  recursion, closure parameter capture after outer return, upstream direct self-recursion through depth 200 and
  direct function/method self-recursive return calls through depth 10,000 without overflowing the host stack, upstream
  anonymous-call parameter adjustment cases,
  `basic.luau` fixed-arity missing/exact/extra argument call rows plus anonymous-call local-binding rows,
  `calls.luau` fixed multi-return lists with call-result adjustment into local/assignment targets,
  including scalar adjustment for non-trailing calls and expansion of the trailing call in value lists,
  trailing-call expansion in return lists including recursive multiple-return propagation through the upstream pattern-match range helper, and trailing
  function-call expansion in enclosing call argument lists, plus `vararg.luau` function declarations,
  vararg expressions, local/return/call-argument list adjustment, method calls with implicit `self` and
  varargs, nested trailing-call and trailing-vararg expansion in enclosing calls, triple nested trailing-vararg
  expansion in return lists, typed type-pack vararg
  annotations that parse and erase at runtime, trailing vararg/function-call expansion in table
  constructors, and table-constructor append calls whose final argument is `...` such as
  `{select(n, ...)}`,
  and `closure.luau` upvalue read/write cases including sibling closures over the same captured cell,
  fresh captured cells for numeric/generic loop control variables per iteration, fresh captured
  cells for locals declared inside repeated loop bodies, and captured-local availability across tail calls
  from vararg functions. Native `assert` now preserves success arguments
  in packed and `select("#", ...)` multi-result call contexts, including nil-bearing result lists. Native
  `select` coverage includes count selectors, positive selectors, valid negative selectors, numeric-string
  selectors, and out-of-range negative errors. Ordinary
  `pcall` success/failure result-list behavior and ordinary `xpcall` success/immediate-handler behavior are
  covered with string-message `error`, including immediate protected-call upvalue closing. Yieldable `pcall`
  coverage includes protected success, nested protected success, and error-after-yield continuation cases;
  yieldable `xpcall` coverage includes protected success, handler success, handler error,
  `debug.traceback` handler stack preservation, and nested
  xpcall/pcall yield-chain cases; protected-call upvalue closing is covered for immediate and deferred
  errors; upstream PCall harness helper coverage includes `cxxthrow` and `resumeerror`; protected
  allocation-limit errors from `table.create(1e6)` are covered through pcall, xpcall, deferred
  coroutine continuations, and `coroutine.close`; self-tail-recursive protected calls now raise
  catchable stack overflow instead of looping forever. Stackless self-recursive non-tail frames are covered
  across coroutine yield/resume, including parent-frame continuation after a yielded base-case return and
  recursive pcall error capture before and after yield; their synthetic frames remain visible to
  `debug.traceback` and `debug.info`. The whole upstream `pcall.luau` script is now
  executable, including Luau-scale ordinary non-tail recursion without overflowing the Rust host stack.
- Upstream Luau 0.709 conformance inventory is mapped in `upstream/IMPORT-PLAN.md` and copied under
  `upstream/luau-0.709/`: 49 scripts, 11,747 lines.
- No row in `design/50-conformance.md` should be considered complete until its fixture is marked
  `luau-0.709` or has a documented exclusion.

## First Upstream Port Batches

Port upstream Luau 0.709 tests in this order:

1. Lexer and parser acceptance/rejection tests.
2. Numeric literal and arithmetic tests, including `%`, `//`, NaN, infinities, and `-0`.
3. Type syntax parse-then-erase tests.
4. Basic runtime control flow and assignment tests.
5. Tables, metatables, and iteration order.
6. Functions, calls, varargs, closures, and upvalues.
7. Error-message fixtures for script-observable errors.
8. Stdlib fixtures, starting with corrected rows from `design/50-conformance.md`: `table.maxn`,
   `table.foreach`, `table.foreachi`, `table.getn`, `getfenv`, `setfenv`, and `math.map`.

## Counting Rule

A conformance topic counts as covered only when:

- it appears in `REFS.tsv`,
- it appears in `MANIFEST.tsv` as a `refs` entry,
- the fixture is executable by the rustia harness,
- the expectation is marked `luau-0.709`, and
- the fixture passes under the intended profile.

- Callable receivers have pinned-oracle controls for ordinary recursive calls, yield/resume, and rejection of non-function `__call` values in ordinary and expanded calls.

The pending-free count is not a universal parity claim. The broader
[release diagnostic audit](../design/94-release-compatibility-audit.md#broader-diagnostic-audit)
records 13 independently reproduced message/prefix differences outside this
manifest, alongside its passing comparisons and exact source cases.
