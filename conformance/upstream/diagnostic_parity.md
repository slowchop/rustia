# Diagnostic parity regression provenance

The fixture `fixtures/upstream-luau-0.709/diagnostic_parity.luau` and Rust
integration test `crates/rustia/tests/diagnostic_parity.rs` are original project
regressions, licensed MIT OR Apache-2.0. They are not copied upstream tests.

Expectations were captured independently from Luau 0.709, upstream commit
`b968ef742741bb2b703afc3b3c53f06608c87481`, through locked `luau0-src`
`0.18.3+luau709`. Build the worker with:

```sh
cargo build --locked --manifest-path fuzz/Cargo.toml --features oracle --bin error-oracle-worker
```

For each of the 85 Rust test cases, wrap its body in the exact single-line
protected-call observer in that test and pass it to
`fuzz/target/debug/error-oracle-worker luau` on standard input. The worker
returns `ok`, `0`, then the observed result. The Rust expectations replace only
the oracle vehicle's `[string "differential.luau"]` source label with the test's
explicit `diagnostic_parity.luau` label. Message text and line numbers remain
unchanged. Returned table/function/thread values use their type names, avoiding
unspecified object addresses.

The complete fixture was also checked independently with:

```sh
fuzz/target/debug/error-oracle-worker luau < conformance/fixtures/upstream-luau-0.709/diagnostic_parity.luau
```

It returns `ok`, `0`, `pass`. The fixture replaces only the arbitrary source
label with `<source>` and preserves exact line numbers. The Rust integration
test compares the entire result without normalization, across four profiles
and before/after reset. Every case is followed by a successful operation to
check recovery.

Coverage includes the thirteen original diagnostic findings, missing versus
explicit nil arguments, aliases, numeric/string coercion, extra arguments,
zero-result trailing expansion, validation precedence, argument side effects,
readonly/protected tables, and direct native `pcall` versus script wrappers.

Relevant pinned source definitions:

- `VM/src/laux.cpp`: `luaL_typeerrorL`, `luaL_checkany`, string/number checks.
- `VM/src/lbaselib.cpp`: `luaB_rawget`, `luaB_rawset`, `luaB_type`, `luaB_next`.
- `VM/src/lstrlib.cpp`: `str_len`, `str_sub`.
- `VM/src/ltablib.cpp`: `tinsert`, `tsort`, `tfreeze`.
- `VM/src/lcorolib.cpp`: `cocreate`, `coresumey`.
- `VM/src/lvmutils.cpp` and `VM/src/ltable.cpp`: invalid length and next-key errors.

These tests cover observed diagnostics, not exhaustive language parity.
