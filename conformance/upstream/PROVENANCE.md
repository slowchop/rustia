# Upstream Luau Test Provenance

Oracle source:

- repository: <https://github.com/luau-lang/luau>
- pinned commit: `b968ef742741bb2b703afc3b3c53f06608c87481`
- upstream ref: `upstream/release/709`
- license: MIT, from upstream `LICENSE.txt`

The upstream conformance suite at this commit has 49 `tests/conformance/*.luau`
scripts, 11,747 total lines. All 49 scripts are copied verbatim under
`conformance/upstream/luau-0.709/tests/conformance/` and tracked in
`conformance/upstream/luau-0.709/PROVENANCE.tsv`.
The upstream conformance runner source is copied at
`conformance/upstream/luau-0.709/tests/Conformance.test.cpp` to document harness
helpers such as the non-verbose `print` override.

Derived executable fixtures:

Detailed source, license, classification, and oracle metadata for these reduced
fixtures is tracked in `conformance/fixtures/upstream-luau-0.709/PROVENANCE.tsv`.
That TSV is the complete derived-fixture index; the examples below summarize the
first reduced fixtures added from the upstream corpus.
Some derived rows cite upstream VM sources by upstream-relative path; those VM
files are not vendored under `conformance/upstream`, and `source-inspection:`
oracle commands mark behavior derived from code inspection instead of a runnable
upstream script.

- `conformance/fixtures/upstream-luau-0.709/assert_returns_first_arg.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert` returns-first-argument behavior.
- `conformance/fixtures/upstream-luau-0.709/assert_nested_number_equality.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(assert(1) == 1)` behavior.
- `conformance/fixtures/upstream-luau-0.709/assert_truthy_number_statement.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(1)` statement.
- `conformance/fixtures/upstream-luau-0.709/assert_truthy_bool_statement.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(true)` statement.
- `conformance/fixtures/upstream-luau-0.709/assert_missing_argument.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert()` missing-argument failure.
- `conformance/fixtures/upstream-luau-0.709/assert_nil_fails.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(nil)` failure message.
- `conformance/fixtures/upstream-luau-0.709/assert_false_fails.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(false)` failure message.
- `conformance/fixtures/upstream-luau-0.709/assert_string_message.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  the upstream `assert(nil, "epic fail")` failure message.
- `conformance/fixtures/upstream-luau-0.709/assert_second_argument_evaluated.luau`
  derives from `tests/conformance/assert.luau` at the pinned commit and covers
  evaluation of later arguments in multi-argument `assert` calls.
- `conformance/fixtures/upstream-luau-0.709/number_not_equal.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  the upstream numeric `~=` behavior.
- `conformance/fixtures/upstream-luau-0.709/number_less_than.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  the upstream numeric `<` behavior.
- `conformance/fixtures/upstream-luau-0.709/number_less_equal.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  the upstream numeric `<=` behavior.
- `conformance/fixtures/upstream-luau-0.709/number_greater_than.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  the upstream numeric `>` behavior.
- `conformance/fixtures/upstream-luau-0.709/number_greater_equal.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  the upstream numeric `>=` behavior.
- `conformance/fixtures/upstream-luau-0.709/not_true.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  unary `not` on `true`.
- `conformance/fixtures/upstream-luau-0.709/not_false.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  unary `not` on `false`.
- `conformance/fixtures/upstream-luau-0.709/not_nil.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  unary `not` on `nil`.
- `conformance/fixtures/upstream-luau-0.709/logical_and_truthy.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  value-preserving `and` when the left operand is truthy.
- `conformance/fixtures/upstream-luau-0.709/logical_and_nil.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  value-preserving `and` when the left operand is falsey.
- `conformance/fixtures/upstream-luau-0.709/logical_or_truthy.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  value-preserving `or` when the left operand is truthy.
- `conformance/fixtures/upstream-luau-0.709/logical_or_nil.luau`
  derives from `tests/conformance/basic.luau` at the pinned commit and covers
  value-preserving `or` when the left operand is falsey.
- `conformance/fixtures/upstream-luau-0.709/print_banner_statement.luau`
  derives from `tests/conformance/assert.luau` and the non-verbose harness
  `print` override in `tests/Conformance.test.cpp` at the pinned commit,
  covering the upstream `print("testing asserts")` banner statement.
- `conformance/fixtures/upstream-luau-0.709/table_freeze_returns_same_and_isfrozen.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  the upstream `table.freeze` identity return plus `table.isfrozen` query behavior.
- `conformance/fixtures/upstream-luau-0.709/table_freeze_shallow_nested_table.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  the upstream shallow-freeze behavior for nested tables.
- `conformance/fixtures/upstream-luau-0.709/table_freeze_blocks_field_write.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  assignment errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/table_freeze_blocks_rawset.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  `rawset` errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/table_freeze_rejects_already_frozen.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  the upstream error when `table.freeze` is called on an already frozen table.
- `conformance/fixtures/upstream-luau-0.709/table_clear_array_hash_length_next.luau`
  derives from `tests/conformance/clear.luau` at the pinned commit and covers
  `table.clear` removing observable array and hash entries.
- `conformance/fixtures/upstream-luau-0.709/table_clear_reuses_array_shape.luau`
  derives from `tests/conformance/clear.luau` at the pinned commit and covers
  post-clear array-part reuse and length behavior.
- `conformance/fixtures/upstream-luau-0.709/table_clear_table_expected_error.luau`
  derives from `tests/conformance/clear.luau` at the pinned commit and covers
  `table.clear` table-argument type error wording.
- `conformance/fixtures/upstream-luau-0.709/table_clear_readonly_error.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  `table.clear` errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/table_insert_readonly_error.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  `table.insert` errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/table_remove_readonly_error.luau`
  derives from `tests/conformance/tables.luau` at the pinned commit and covers
  `table.remove` errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/table_move_readonly_error.luau`
  derives from `tests/conformance/move.luau` at the pinned commit and covers
  `table.move` errors on readonly destination tables.
- `conformance/fixtures/upstream-luau-0.709/table_move_source_table_expected_error.luau`
  derives from `tests/conformance/move.luau` at the pinned commit and covers
  the source-table type error wording for `table.move`.
- `conformance/fixtures/upstream-luau-0.709/table_move_destination_table_expected_error.luau`
  derives from `tests/conformance/move.luau` at the pinned commit and covers
  the explicit destination-table type error wording for `table.move`.
- `conformance/fixtures/upstream-luau-0.709/table_sort_readonly_error.luau`
  derives from `tests/conformance/sort.luau` at the pinned commit and covers
  `table.sort` errors on readonly tables.
- `conformance/fixtures/upstream-luau-0.709/string_pack_integer_sizes_unpack.luau`,
  `string_pack_endian_alignment.luau`, `string_pack_strings_errors.luau`, and
  `string_pack_direct_edges.luau`
  derive from `tests/conformance/tpack.luau` at the pinned commit and cover
  representative `string.pack`, `string.unpack`, and `string.packsize` integer,
  endian, alignment, string, and error behavior.
- `conformance/fixtures/upstream-luau-0.709/string_literal_high_byte_escapes.luau`
  derives from `tests/conformance/strings.luau` at the pinned commit and covers
  decimal and hexadecimal high-byte string escapes.
- `conformance/fixtures/upstream-luau-0.709/string_byte_range_results.luau`
  derives from `tests/conformance/strings.luau` at the pinned commit and covers
  `string.byte` returning one result per selected byte.
- `conformance/fixtures/upstream-luau-0.709/number_binary_literal.luau`
  derives from `tests/conformance/buffers.luau` at the pinned commit and covers
  Luau binary numeric literal syntax without requiring the full buffer stdlib.
- `conformance/fixtures/upstream-luau-0.709/bit32_basic_folds.luau`,
  `bit32_shift_rotate.luau`, `bit32_extract_replace_count_byteswap.luau`, and
  `bit32_coercion_and_errors.luau`, and `bit32_conversion_edges.luau` derive from `tests/conformance/bitwise.luau`
  at the pinned commit and cover the Luau `bit32` runtime surface.
- `conformance/fixtures/upstream-luau-0.709/stackless_recursive_debug_stack.luau`
  derives from `tests/conformance/debug.luau` at the pinned commit and covers
  recursive non-tail Luau frames remaining visible through `debug.traceback` and `debug.info`.

When porting starts, each entry must record:

- upstream repository URL
- upstream commit, pinned to `b968ef742741bb2b703afc3b3c53f06608c87481`
- original path
- local path
- license/provenance note
- classification (`api-native`, `core-language`, `parser`, `parser-runtime`, `runtime`, `stdlib`,
  `error-message`, `determinism`, or `excluded`)
- oracle command used to derive expected output

## Release compatibility audit regressions

`fixtures/upstream-luau-0.709/metamethod_yield_errors.luau` is an original
Rustia regression (project licence), not copied upstream code. It tests runtime
error behaviour against https://github.com/luau-lang/luau at pinned commit
`b968ef742741bb2b703afc3b3c53f06608c87481`, `VM/src/ldo.cpp`.
Classification: `error-message`. Oracle capture:
`fuzz/target/debug/error-oracle-worker luau < conformance/fixtures/upstream-luau-0.709/metamethod_yield_errors.luau`
returns `ok`, `0`, `pass` after building the locked fuzz worker with `--features oracle`.

`fixtures/upstream-luau-0.709/method_callee_evaluation_order.luau` is also an
original project-licensed regression, classified `runtime`. Its initial 14 cases were
individually captured against the same pin, then the complete assertion fixture
was run with `fuzz/target/debug/error-oracle-worker luau < conformance/fixtures/upstream-luau-0.709/method_callee_evaluation_order.luau`,
returning `ok`, `0`, `pass`. Relevant source: `Compiler/src/Compiler.cpp`.

The same method fixture now includes eight exact missing-method message suffixes
(the oracle vehicle wraps source names differently), and six receiver, argument
failure, GC, protected-resume and append-vararg cases. The complete extended
fixture again returns `ok`, `0`, `pass` through the pinned oracle.
