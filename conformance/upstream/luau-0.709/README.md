# Luau 0.709 upstream tests

Rustia targets [Luau 0.709](https://github.com/luau-lang/luau/tree/b968ef742741bb2b703afc3b3c53f06608c87481). The upstream code is [MIT-licensed](LICENSE.txt).

## Runtime scripts

All 49 scripts in [tests/conformance](tests/conformance) are copied verbatim and run through the Rustia
conformance harness. [PROVENANCE.tsv](PROVENANCE.tsv) identifies the upstream test for each script.

## Parser tests

The [case inventory](parser-cases.tsv) records every test declaration in the pinned
[Parser.test.cpp](https://github.com/luau-lang/luau/blob/b968ef742741bb2b703afc3b3c53f06608c87481/tests/Parser.test.cpp).
There are 235 declarations. Two pairs share a name, so each entry also gives its upstream line.

| Coverage at the #35 audit | Entries | What the evidence column tells you |
| --- | ---: | --- |
| Existing | 12 | Which earlier Rustia test already checks the applicable behavior. |
| Ported | 189 | Which test adds or strengthens a check. One upstream case can contain several programs or assertions. |
| Outside | 34 | Why the upstream behavior has no equivalent in Rustia, such as declaration-file analysis or C++ arena allocation. |

A row can mix public behavior and upstream internals. Rustia returns its first diagnostic, for example.
The inventory identifies where a port checks that error while Luau also returns a partial tree or later errors.
Rustia checks ordinary loop control and vararg use at compilation. The corresponding rows cite compiler tests.

Run the Rustia assertions with `cargo test --locked -p rustia-parser --test luau_0709`. The same inputs
also run through the pinned Luau compiler in hosted CI. That comparison checks which programs are accepted.
The Rust assertions check the values, tree shapes, source spans and diagnostics listed in the inventory.
The reference compiler enables Luau's explicit type argument syntax to match Rustia. Run the comparison with
`cargo test --locked --manifest-path fuzz/Cargo.toml --features oracle --test upstream_parser`.

## Lexer tests

The [lexer inventory](lexer-cases.tsv) maps all 28 declarations in the pinned
[Lexer.test.cpp](https://github.com/luau-lang/luau/blob/b968ef742741bb2b703afc3b3c53f06608c87481/tests/Lexer.test.cpp).
Two names each appear twice upstream. Their line numbers distinguish string cases from comment cases.
Twelve had coverage for their applicable behavior before this audit. Sixteen gained new or stronger checks.
The tests use every upstream input, including the twelve with prior coverage.

The [lexer tests](../../../crates/rustia-lexer/tests/luau_0709.rs) compare token order, decoded bytes,
source spans and EOF. Rustia skips complete comments and returns a vector of tokens. Luau also exposes
comment tokens, quote style, delimiter depth and a streaming lookahead cursor.
The [interpolation tests](../../../crates/rustia-parser/tests/luau0709/lexer_interpolation.rs) check
literal sections, embedded expressions and the first error. Luau's additional recovery tokens have
no equivalent in Rustia.

Run the direct ports with `cargo test --locked -p rustia-lexer --test luau_0709`. Interpolated strings
are handled by the parser. Run those six ports with
`cargo test --locked -p rustia-parser --test luau_0709 lexer_interpolation`. They also run in the
shared pinned-compiler comparison described above.
