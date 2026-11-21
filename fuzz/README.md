# rustia fuzzing

This independent Cargo workspace contains bounded `cargo-fuzz` targets for the lexer, parser,
compiler, VM, stateful embedding, and differential execution.

The [compiler case tests](tests/upstream_compiler.rs) also run as a fixed hosted CI suite.
Run them locally with `cargo test --locked --manifest-path fuzz/Cargo.toml --features oracle --test upstream_compiler`.
The [case inventory](../conformance/upstream/luau-0.709/compiler-cases.tsv) links each upstream declaration to evidence.

| target | input bound | additional bounds |
| --- | ---: | --- |
| `lexer` | 64 KiB | raw bytes through `lex_bytes` |
| `parser` | 32 KiB | raw bytes through `parse_source_bytes` |
| `compiler` | 16 KiB | only successfully parsed chunks reach `compile` |
| `vm` | 2 KiB | 256 decoded instructions, 16 registers, fixed small constants, one prototype, recursion depth 32, and 4,096 fuel |
| `stateful` | 128 bytes | bounded API sequences, retained exports/functions, object replacement/aliases/reset, 2 MiB guest quota, 4,096 fuel per execution |
| `differential` | 64 bytes | generated loop-free source, 8 MiB guest quotas, 100,000 Rustia fuel, isolated five-second oracle worker |

The VM decoder covers every production opcode with a 512 KiB logical-live quota.
It exercises bytecode validation, control flow, registers, arithmetic, tables, calls,
closures, imports, inline-cache instructions, and all four runtime profiles.

The stateful target has 25 operations. Its retained-handle model checks current
and previous exports plus one detached function after every operation, including
successful replacement, a rejected installation that first mutates captured
state, cloned/dropped roots, collection and reset. A weak witness proves actual
collection under the quota in every profile. One stale export/function pair
remains across later allocations to check the exact reset epochs and prevent
handle revival. Live and stale slots are bounded; guest input is still limited
to 128 bytes. Explicit lifecycle traces and seeded mutation runs execute in the
ordinary fuzz-workspace tests; they require no Luau worker or nightly toolchain.

The differential worker pins `mlua` 0.11.6 and `luau0-src` 0.18.3+luau709 in
this independent lockfile. Build it with `cargo build --locked --manifest-path
fuzz/Cargo.toml --features oracle --bin error-oracle-worker`, then set
`RUSTIA_DIFF_WORKER` to its absolute executable path. Run the explicit
`generated_differential_smoke` ignored test with that environment to exercise
the deterministic generated corpus. Normal fuzz-workspace tests need no oracle.

The generated differential corpus has 61 bounded shapes. CI smoke derives its
shape range from the generator's `SOURCE_SHAPE_COUNT` and includes all eight type
variants: valid and invalid single/grouped/tuple assertions, alias default ordering,
pack defaults, generic function binders/named parameters/final tails, and assertion result adjustment. These cases compare acceptance or
rejection with the oracle before observing execution. Raw frontend fuzzing remains
a crash/robustness check; neither it nor this targeted generator covers all type grammar.

The [Error behaviour and fuzzing workflow](../.github/workflows/error-fuzz.yml)
ran the differential smoke at public launch and also runs it on PRs and manual
dispatch. Only manual dispatch starts the longer campaigns. It runs 30 minutes per target, then two additional hours for
stateful and differential. It retains revision, toolchain, seed-bearing logs,
corpora and findings. These are correctness runs, not performance evidence.
Timeouts, abnormal worker exits and mismatches require triage. The current
generator compares frontend rejection versus execution (not internal parser
versus compiler phases), side effects and explicit protected-call observations;
resource-policy equivalence is deliberately not inferred from Luau results.

Completed campaign revisions, execution counts, the fixed PUC table-key finding,
and consumer acceptance are recorded in the
[error behaviour evidence](../design/82-error-behaviour-fuzzing.md).
A clean bounded campaign is not proof that no bugs remain.

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

## Upstream parser examples

Run `cargo test --locked --manifest-path fuzz/Cargo.toml --features oracle --test upstream_parser`
to compare the ported examples with Luau 0.709. The [inventory](../conformance/upstream/luau-0.709/parser-cases.tsv)
identifies which Rustia assertions check each applicable upstream case.
