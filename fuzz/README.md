# rustia fuzzing

This independent Cargo workspace contains bounded `cargo-fuzz` targets for the lexer, parser,
compiler, and VM.

| target | input bound | additional bounds |
| --- | ---: | --- |
| `lexer` | 64 KiB | raw bytes through `lex_bytes` |
| `parser` | 32 KiB | raw bytes through `parse_source_bytes` |
| `compiler` | 16 KiB | only successfully parsed chunks reach `compile` |
| `vm` | 2 KiB | 256 decoded instructions, 16 registers, fixed small constants, one prototype, recursion depth 32, and 4,096 fuel |

The VM decoder excludes operations that can request arbitrary standard-library allocation sizes.
It still exercises bytecode validation, control flow, registers, arithmetic, tables, calls,
closures, imports, inline-cache instructions, and all three runtime profiles. Expand that opcode
set after the VM has a hard memory quota.

Install `cargo-fuzz` once, then run a target from the repository root:

```sh
cargo +nightly fuzz run lexer -- -max_len=65536 -timeout=5
cargo +nightly fuzz run parser -- -max_len=32768 -timeout=5
cargo +nightly fuzz run compiler -- -max_len=16384 -timeout=5
cargo +nightly fuzz run vm -- -max_len=2048 -timeout=5
```

Use `-max_total_time=<seconds>` for a bounded smoke campaign. Seed inputs live under
`corpus/<target>/`; minimized crashes are written under `artifacts/<target>/` and remain ignored.

The deterministic harness tests do not require nightly:

```sh
cargo test --manifest-path fuzz/Cargo.toml
```
