# Rustia

**Luau scripting for Rust applications, with reproducible execution when you need it.**

Use Rustia to add game logic, configuration, or user-written scripts to your application.
Pass Rust values into a script, expose Rust functions, and read typed results back.
Rustia implements its interpreter in Rust and runs on native platforms and WebAssembly.

**Pre-release:** the crates are not published yet, and the API can still change.
We're working toward a stable first release; see the [release milestone](https://github.com/slowchop/rustia/blob/main/design/92-first-release.md).

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
    let answer: f64 = vm
        .load("return 40 + 2")
        .source_name("answer.luau")
        .eval()?;

    assert_eq!(answer, 42.0);
    println!("The answer is {answer}");
    Ok(())
}
```

The Rust return type chooses the conversion. Strings, booleans, optional values,
vectors, and string-keyed maps work too. `source_name` gives errors a useful filename;
`vm.eval("return 40 + 2")` is the shorter form when you don't need one.

### Call Rust from a script

```rust
use rustia::Rustia;

fn main() -> rustia::Result<()> {
    let mut vm = Rustia::builder().build()?;
    vm.insert_global("player_name", "Ada")?;
    vm.add_fn("double", |value: f64| value * 2.0)?;

    let greeting: String = vm.eval("return 'Hello, ' .. player_name")?;
    let score: f64 = vm.eval("return double(21)")?;

    assert_eq!(greeting, "Hello, Ada");
    assert_eq!(score, 42.0);
    Ok(())
}
```

Callbacks can return `rustia::Result<T>` when an operation may fail. For larger APIs,
group functions into a `HostModule`; use `create_function` when scripts should own
the lifetime of a callback. See the [host function guide](https://github.com/slowchop/rustia/blob/main/design/78-host-functions.md).

### Call a script function from Rust

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

`install` runs a candidate module before handing you its exports. If installation
fails, Rustia restores the previous guest state. Keep the old exports until a new
installation succeeds to support [hot reload](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs).
External actions performed by Rust callbacks are outside this rollback.

## Choose an environment

`Rustia::builder()` selects the standard environment. Choose another preset when
your application needs real clocks or strict replay:

| Preset | Use it for | Defaults |
|---|---|---|
| `RustiaBuilder::standard()` | General embedding | Fixed random seed and repeatable clocks; real stdout on native/WASI |
| `RustiaBuilder::live()` | Everyday scripting with real-world inputs | System clocks, random seed, output, and runtime compilation |
| `RustiaBuilder::deterministic()` | Simulations and replay | Repeatable execution; rejects host services declared nondeterministic |
| `RustiaBuilder::deterministic_leaf()` | Isolated, stateless decisions | Smaller frozen script environment; explicit memory limit required |

Give a script an instruction budget and a VM-owned memory limit:

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

“Fuel” is an instruction-based budget rather than a time limit. The memory limit covers
Rustia's accounted VM allocations, not the application's total process memory.
Host callbacks need their own bounds and must honor their declared replay behavior.
`standard()` permits explicitly added live services; `live()` promises no reproducibility.
See the [embedding](https://github.com/slowchop/rustia/blob/main/design/75-embedded-api.md) and
[host services](https://github.com/slowchop/rustia/blob/main/design/72-host-services.md) guides.

## What you get

- **Familiar scripting:** functions, tables, closures, coroutines, and standalone
  Luau libraries, targeting [Luau 0.709](https://github.com/slowchop/rustia/blob/main/design/05-luau-target.md).
- **Rust-friendly data:** typed values and callbacks, Serde conversion, and live
  tables/functions when you need shared or cyclic data.
- **Useful errors:** source names, highlighted expressions, call traces, typed error
  categories, and paths into data that failed to deserialize.
- **Control over execution:** instruction and memory budgets, host-controlled module
  loading, and explicit reset for repeated decisions.
- **One portable runtime:** native and `wasm32-unknown-unknown`, with browser service
  adapters available through the default `web-live` feature.
- **Safe Rust in our production code:** workspace crates forbid `unsafe` code.

Rustia targets standalone Luau. It does not include Roblox services or act as Luau's
static type checker. The current suite has **712 fixtures** covering **511 named
behaviors**, with **0 pending** cases; that is a tested boundary rather than a claim
to match every Luau program. See [conformance coverage](https://github.com/slowchop/rustia/blob/main/conformance/COVERAGE.md).

## How does it compare?

Choose by the language and integration your application needs:

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
The [claim checks](https://github.com/slowchop/rustia/blob/72fc97845ca82dabe473ddf994a8d66d6aa05b06/bench/CLAIMS.md) approve no overall speed headline from this snapshot.
Fresh measurements run in CI, never on a contributor's local machine.

## Run scripts from the command line

From a checkout:

```sh
cargo run --locked -p rustia-cli -- path/to/script.luau
cargo run --locked -p rustia-cli -- --eval "return 40 + 2"
cargo run --locked -p rustia-cli -- --repl
```

The CLI resolves `require` relative to the calling file in standard and compatibility
profiles. The deterministic profile leaves filesystem access to an explicitly supplied
host resolver. See the [module loading example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/module_resolution.rs).

## Errors you can show to people

```rust
use rustia::Rustia;

fn main() -> rustia::Result<()> {
    let mut vm = Rustia::builder().build()?;
    if let Err(error) = vm
        .load("return missing.health")
        .source_name("combat.luau")
        .exec()
    {
        eprintln!("{}", error.report());
    }
    Ok(())
}
```

Use `error.report()` for a console or log and `error.kind()` for program decisions.
See the [diagnostics example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/diagnostics.rs).

## Explore and contribute

Run an example with `cargo run --locked -p rustia --example embedding`.
Other starting points: [live tables](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/live_values.rs),
[custom environments](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/chunk_builder.rs),
[typed host transactions](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/typed_invocation.rs), and the
[Koto migration guide](https://github.com/slowchop/rustia/blob/main/design/76-koto-migration.md).

The [first-release milestone](https://github.com/slowchop/rustia/blob/main/design/92-first-release.md) and
[release checklist](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md) track API stability,
remaining compatibility work, platform validation, and publication. The
[canonical project state](https://github.com/slowchop/rustia/blob/main/design/CURRENT.md) has the engineering record;
[CONTRIBUTING.md](https://github.com/slowchop/rustia/blob/main/CONTRIBUTING.md) explains how to build, test, and help.
Production code lives in `crates/`, compatibility tests in `conformance/`, design
notes in `design/`, and historical experiments in `spikes/`.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions are dual-licensed under those same terms unless explicitly stated otherwise.
