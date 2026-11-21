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
