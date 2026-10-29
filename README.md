# Rustia

Rustia is a Luau interpreter written in Rust. Use it to add game logic,
configuration, or user-written scripts to a Rust application, on native platforms
or WebAssembly.

Call Rust functions from scripts, read results as Rust types, and choose whether
scripts use real clocks and randomness or run reproducibly for simulations and replay.

Rustia is **pre-release and currently private**. Packages are unpublished at
0.0.1, and the API can still change.
Source availability is separate from a supported release. Compatibility claims
cover the tested behavior in the conformance corpus, not full Luau parity on
every platform. See the [platform evidence](https://github.com/slowchop/rustia/blob/main/design/96-browser-platform-acceptance.md)
for executed tests and remaining gaps. Contribution and support policies are
undecided; source access does not promise either service.

## Try it

Add this to your application's `Cargo.toml` (Rust 1.95 or newer):

```toml
[dependencies]
rustia = { git = "https://github.com/slowchop/rustia", rev = "b84b5f6715e31756f00d1f39e72c8246f179b174" }
```

This pin selects the verified cleaned-source baseline. Historical benchmark
source IDs remain unchanged in the evidence.

Put this in `src/main.rs` and run `cargo run`:

```rust
use rustia::Rustia;

fn main() -> rustia::Result<()> {
    let mut vm = Rustia::builder().build()?;
    vm.insert_global("base_score", 21)?;
    vm.add_fn("double", |value: f64| value * 2.0)?;

    let score: f64 = vm
        .load("return double(base_score)")
        .source_name("score.luau")
        .eval()?;

    println!("The score is {score}");
    assert_eq!(score, 42.0);
    Ok(())
}
```

This prints:

```text
The score is 42
```

The return type chooses the conversion. Strings, booleans, optional values,
vectors, and string-keyed maps work too. For a quick expression,
`vm.eval("return 40 + 2")` is the shorter form. Rust callbacks can return
`rustia::Result<T>` when an operation may fail.

## Choose how to run a script

| Need | Use |
| --- | --- |
| Run code without returning data to Rust | `vm.load(source).exec()` |
| Copy a result into a Rust type | `vm.load(source).eval::<T>()` |
| Keep module exports and replace them only after successful loading | `vm.load(source).install()` |

Ordinary `exec` and `eval` calls keep guest mutations completed before an error.
`install` restores the prior guest state if installation fails. Neither option
undoes external actions performed by Rust callbacks.

## Call script functions from Rust

A script can return functions for the application to call:

```rust
use rustia::Rustia;

fn main() -> rustia::Result<()> {
    let mut vm = Rustia::builder().build()?;
    let exports = vm
        .load("return { greet = function(name) return 'Hello, ' .. name end }")
        .source_name("greetings.luau")
        .install()?;

    let greet = exports.function(&mut vm, "greet")?;
    let message: String = greet.call(&mut vm, ("Ada",))?;
    assert_eq!(message, "Hello, Ada");
    Ok(())
}
```

`install` runs the new module before returning its exports. If it fails, Rustia
restores the previous script state. Keep the old exports until a replacement
succeeds to support [hot reload](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs).
External actions performed by Rust callbacks are outside this rollback.

For structured data, use `eval_serde` with Serde. Your application also needs
`serde = { version = "1", features = ["derive"] }` in its dependencies to derive
Rust data types. For shared or cyclic tables, keep
[live table and function values](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/live_values.rs)
in the VM instead of copying them into Rust collections.

## Choose an environment

`Rustia::builder()` selects the standard environment:

| Preset | Use it for | Defaults |
|---|---|---|
| `RustiaBuilder::standard()` | General embedding | Fixed random seed and repeatable clocks; real stdout on native/WASI |
| `RustiaBuilder::live()` | Scripts that need real-world inputs | System clocks, random seed, output, and runtime compilation |
| `RustiaBuilder::deterministic()` | Simulations and replay | Repeatable execution; rejects host services declared nondeterministic |

The ordinary builder has no instruction-fuel or accounted-memory limit by
default. Choose explicit budgets when you need bounded guest execution:

```rust
use rustia::RustiaBuilder;

fn main() -> rustia::Result<()> {
    let mut vm = RustiaBuilder::deterministic()
        .fuel(100_000)
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()?;

    let answer: f64 = vm.eval("return 6 * 7")?;
    assert_eq!(answer, 42.0);
    Ok(())
}
```

Fuel gives scripts an instruction-based budget. The memory limit covers Rustia's
accounted VM allocations, not the application's total memory. Rust callbacks
need their own bounds and must honor their declared replay behavior.
`standard()` permits explicitly added live services; `live()` promises no reproducibility.

## Examples and API reference

To read the API reference locally:

```sh
git clone https://github.com/slowchop/rustia.git
cd rustia
cargo doc --locked -p rustia --no-deps --open
```

Choose an example by the task you want to try. These use the application API;
you do not need to read the VM design documents first.

| Task | Example |
| --- | --- |
| Run scripts, pass data and call Rust functions | [Embedding](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/embedding.rs) |
| Give scripts methods on Rust objects | [Game objects](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/game_objects.rs) or [mutable objects](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/mutable_objects.rs) |
| Keep shared tables and script functions alive | [Live values](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/live_values.rs) |
| Load modules through a Rust resolver | [Module resolution](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/module_resolution.rs) |
| Replace a script while keeping the old one if loading fails | [Hot reload](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs) |
| Choose globals and a script environment | [Chunk builder](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/chunk_builder.rs) |
| Borrow application state and collect staged changes | [Typed invocation](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/typed_invocation.rs) |
| Show source-aware script errors | [Diagnostics](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/diagnostics.rs) |

Run an example with `cargo run --locked -p rustia --example embedding`.
Examples with `_bench` in their name are measurement tools; do not run them on
a development machine.

Use `error.report()` for source-aware diagnostics and `error.kind()` to decide
how your application responds. The
[diagnostics example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/diagnostics.rs)
shows the formatted output.

To run a script without writing a Rust application:

```sh
cargo run --locked -p rustia-cli -- path/to/script.luau
cargo run --locked -p rustia-cli -- --eval "print(40 + 2)"
cargo run --locked -p rustia-cli -- --repl
cargo run --locked -p rustia-cli -- --version
```

The `--eval` example prints `42`. Script return values are discarded by the
CLI; use `print` for output. In the REPL, enter `=40 + 2` to display an expression.

## How does it compare?

Rustia targets standalone Luau 0.709, including functions, tables, closures,
coroutines, and its standard libraries. It does not include Roblox services or
Luau's static type checker. The [conformance coverage](https://github.com/slowchop/rustia/blob/main/conformance/COVERAGE.md)
records the behavior tested against the reference interpreter.

Rustia's production workspace forbids unsafe Rust. It provides typed callbacks,
Serde conversion, source diagnostics, and instruction and memory limits on native
platforms and WebAssembly. Choose an engine by the language and integration you need:

| Engine | Script language | Features to weigh |
|---|---|---|
| **Rustia** | Luau 0.709 target | Rust interpreter, replay modes, instruction budgets, decision reset, typed Rust objects/callbacks and Serde; pre-release |
| **[mlua](https://github.com/mlua-rs/mlua)** | Lua or Luau, selected at build time | Upstream runtimes; custom Rust userdata, scoped callbacks, Serde, and async support ([API](https://docs.rs/mlua/latest/mlua/)) |
| **[Koto](https://koto.dev/)** | Koto | Exported functions, custom Rust objects, and Serde ([guide](https://koto.dev/docs/0.16/api/)) |
| **[Rhai](https://rhai.rs/book/)** | Rhai | Rust-like scripting with [custom Rust types](https://rhai.rs/book/rust/custom-types.html) and [operation limits](https://rhai.rs/book/safety/max-operations.html) |

Expose a Rust type's methods with `ObjectType<T>` and keep instances as live
`Object<T>` handles. Deterministic methods read input and stage requests;
standard/live methods can change application state immediately. See the
[object guide](https://github.com/slowchop/rustia/blob/main/design/83-rust-objects.md)
for an example and lifetime rules. Custom fields/metamethods, scoped callbacks,
and async host calls remain future work.

### Speed: a measured snapshot

**Microseconds per work unit; lower is better.** These are best observed
steady-state interpreter times on Linux x86-64, measured by `ci-bench` on
2026-10-22 UTC at source commit `9b12a5a`. Compilation and first-call costs are separate.

<!-- rustia-benchmark-snapshot
{
  "commit": "9b12a5a8540f69fe7c0f6daebe467c795bb576c3",
  "target": "linux-x86_64",
  "start": 1792642181,
  "end": 1792642445,
  "engines": [
    "rustia",
    "mlua-luau",
    "koto",
    "rhai"
  ],
  "workloads": [
    "dense_tables",
    "sparse_tables",
    "closures_upvalues",
    "generic_iteration",
    "stdlib_math_table"
  ]
}
-->
| Work unit | Rustia | mlua + Luau | Koto | Rhai |
|---|---:|---:|---:|---:|
| Build, sum, and sort a 256-item array | 51.657 | 18.919 | 262.217 | 313.191 |
| Fill and read 128 hash entries | 87.555 | 22.485 | 146.082 | 348.085 |
| One closure call updating a captured value | 0.197 | 0.051 | 0.346 | 1.478 |
| Custom, keyed, and array iteration pass | 14.035 | 4.170 | 7.391 | 9.566 |
| Math library calls and buffered appends | 1.421 | 0.237 | 1.536 | 3.174 |

These workloads exercise different languages and runtime features, so they don't
provide a universal ranking. Rustia trails the upstream Luau interpreter in each
of these tests. The [full results](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/RESULTS.md) include more engines, JIT modes,
startup costs, and [raw snapshot provenance](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/current-results.json).
See the [benchmark limitations](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/CLAIMS.md) for the scope of these comparisons.

## Project status and source access

Source preparation: this cleaned-history candidate is not yet public. Packages
remain unpublished at 0.0.1. Historical benchmark records retain their original
source IDs through the history projection (`design/history-projection.json`);
redacted metadata has separate hashes and original commit signatures do not
survive the rewrite. The prepared hosted CI checks correctness; new performance
measurements await a trusted private bridge. See `design/98-public-source-readiness.md` in the source checkout.

Source visibility does not establish an outside contribution or support policy.
That policy is undecided; this repository does not currently invite pull
requests or promise issue review. Issues remain disabled during private staging.

[Maintainer build and validation notes](https://github.com/slowchop/rustia/blob/main/CONTRIBUTING.md)
explain the development workflow. The [first-release milestone](https://github.com/slowchop/rustia/blob/main/design/92-first-release.md)
and [release checklist](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md)
track API, compatibility, and platform work before a supported release.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
