# Rustia

Rustia is a Luau interpreter written in Rust. Use it to add game logic,
configuration, or user-written scripts to a Rust application, on native platforms
or WebAssembly.

Call Rust functions from scripts, read results as Rust types, and choose whether
scripts use real clocks and randomness or run reproducibly for simulations and replay.

Rustia is **pre-release**. The crates are not published yet, and the API can still change.

## Try it

Add this to your application's `Cargo.toml` (Rust 1.95 or newer):

```toml
[dependencies]
rustia = { git = "https://github.com/slowchop/rustia", rev = "7438a053a1d56987cd124cdc8e9f15001c65d753" }
```

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

For structured data, use Serde. For shared or cyclic tables, keep
[live table and function values](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/live_values.rs)
in the VM instead of copying them into Rust collections.

## Choose an environment

`Rustia::builder()` selects the standard environment:

| Preset | Use it for | Defaults |
|---|---|---|
| `RustiaBuilder::standard()` | General embedding | Fixed random seed and repeatable clocks; real stdout on native/WASI |
| `RustiaBuilder::live()` | Scripts that need real-world inputs | System clocks, random seed, output, and runtime compilation |
| `RustiaBuilder::deterministic()` | Simulations and replay | Repeatable execution; rejects host services declared nondeterministic |

Set an instruction budget and a memory limit when scripts need bounded resources:

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
| **Rustia** | Luau 0.709 target | Rust interpreter, replay modes, instruction budgets, decision reset, typed callbacks and Serde; pre-release |
| **[mlua](https://github.com/mlua-rs/mlua)** | Lua or Luau, selected at build time | Upstream runtimes; custom Rust userdata, scoped callbacks, Serde, and async support ([API](https://docs.rs/mlua/latest/mlua/)) |
| **[Koto](https://koto.dev/)** | Koto | Exported functions, custom Rust objects, and Serde ([guide](https://koto.dev/docs/0.16/api/)) |
| **[Rhai](https://rhai.rs/book/)** | Rhai | Rust-like scripting with [custom Rust types](https://rhai.rs/book/rust/custom-types.html) and [operation limits](https://rhai.rs/book/safety/max-operations.html) |

Rustia's public userdata registration, scoped callbacks, and async host calls are
still unavailable. If those are essential today, weigh the alternatives above.

### Speed: a measured snapshot

**Microseconds per work unit; lower is better.** These are best observed
steady-state interpreter times on Linux x86-64, measured by `ci-bench` on
2026-10-11 UTC at source commit `343c261`. Compilation and first-call costs are separate.

<!-- rustia-benchmark-snapshot
{
  "commit": "343c2615e6f2a38b918258c86c5c7dac0f2ca7fe",
  "target": "linux-x86_64", "start": 1791747073, "end": 1791747336,
  "engines": ["rustia", "mlua-luau", "koto", "rhai"],
  "workloads": ["dense_tables", "sparse_tables", "closures_upvalues", "generic_iteration", "stdlib_math_table"]
}
-->
| Work unit | Rustia | mlua + Luau | Koto | Rhai |
|---|---:|---:|---:|---:|
| Build, sum, and sort a 256-item array | 51.980 | 19.158 | 263.146 | 327.449 |
| Fill and read 128 hash entries | 87.426 | 21.581 | 144.268 | 351.553 |
| One closure call updating a captured value | 0.176 | 0.051 | 0.341 | 1.468 |
| Custom, keyed, and array iteration pass | 13.166 | 4.297 | 7.393 | 9.580 |
| Math library calls and buffered appends | 1.200 | 0.237 | 1.517 | 3.196 |

These workloads exercise different languages and runtime features, so they don't
provide a universal ranking. Rustia trails the upstream Luau interpreter in each
of these tests. The [full results](https://github.com/slowchop/rustia/blob/72fc97845ca82dabe473ddf994a8d66d6aa05b06/bench/RESULTS.md) include more engines, JIT modes,
startup costs, and [raw snapshot provenance](https://github.com/slowchop/rustia/blob/72fc97845ca82dabe473ddf994a8d66d6aa05b06/bench/current-results.json).
See the [benchmark limitations](https://github.com/slowchop/rustia/blob/72fc97845ca82dabe473ddf994a8d66d6aa05b06/bench/CLAIMS.md) for the scope of these comparisons.

## Examples and API reference

To read the API reference locally:

```sh
git clone https://github.com/slowchop/rustia.git
cd rustia
cargo doc --locked -p rustia --no-deps --open
```

The [examples](https://github.com/slowchop/rustia/tree/main/crates/rustia/examples)
cover [embedding](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/embedding.rs),
[host-controlled module loading](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/module_resolution.rs),
[custom environments](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/chunk_builder.rs),
and [host transactions](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/typed_invocation.rs).
Run one with `cargo run --locked -p rustia --example embedding`.

Use `error.report()` for source-aware diagnostics and `error.kind()` to decide
how your application responds. The
[diagnostics example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/diagnostics.rs)
shows the formatted output.

To run a script without writing a Rust application:

```sh
cargo run --locked -p rustia-cli -- path/to/script.luau
cargo run --locked -p rustia-cli -- --eval "return 40 + 2"
cargo run --locked -p rustia-cli -- --repl
```

## Contributing

[CONTRIBUTING.md](https://github.com/slowchop/rustia/blob/main/CONTRIBUTING.md)
explains how to build, test, and help. The [first-release milestone](https://github.com/slowchop/rustia/blob/main/design/92-first-release.md)
and [release checklist](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md)
track the remaining API, compatibility, and platform work before publication.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions are dual-licensed under those same terms unless explicitly stated otherwise.
