# Rustia

Rustia is a pure-Rust interpreter for standalone Luau. Use it to script a Rust
application or replay a simulation on native and WebAssembly targets.

**Deterministic replay.** The same Luau script runs natively on the left and as
WebAssembly in a browser on the right. With the same random seed, each racer
takes the same steps on both tracks. Change the seed and the winner changes
from Mint to Iris. [Record the race](https://github.com/slowchop/rustia/blob/main/demos/web-live/README.md#record-the-race).

![Native and WebAssembly replays side by side. Mint wins with seed 7 and Iris wins with seed 11 on both.](https://github.com/slowchop/rustia/raw/fdcb4bae6d376bc6f323cc47b70521479008133a/demos/web-live/race.gif)

## Try it

You’ll need Rust 1.95 or newer and access to the private repository. To try Rustia
before its first crate release, run `cargo new hello-rustia`, then `cd hello-rustia`.
Add this line under `[dependencies]` in `Cargo.toml`.

```toml
rustia = { git = "https://github.com/slowchop/rustia", rev = "bfe174e1948259be57807ca10aee3b4ff55f9f56" }
```

Rust gives Luau a score and a function called `double`. Put this in
`src/main.rs` and run `cargo run`.

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

The program prints `The score is 42`. Use `eval` to get a value back from Luau,
or `exec` when you just want to run the script.

## Call script functions from Rust

Replace `src/main.rs` with this example and run `cargo run` again. This time
Luau defines `greet`, and Rust calls it with `"Ada"`. The program prints `Hello, Ada`.

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
    println!("{message}");
    assert_eq!(message, "Hello, Ada");
    Ok(())
}
```

To reload a script, keep its functions until the edited version installs
successfully. If the edit fails, your app can keep calling the old functions.
[Hot reload example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs)

**Hot reload in the browser.** The racer keeps moving when an edit introduces a syntax
error. Changing the rule from `+ 2` to `+ 4` doubles its speed on the next tick. [Run the demo](https://github.com/slowchop/rustia/blob/main/demos/web-live/README.md#edit-a-running-race).

![A running Luau script keeps its last working rule, then changes from two to four metres per tick.](https://github.com/slowchop/rustia/raw/9fd5cd6162713c356472c99631c3437f5514e92c/demos/web-live/hot-reload.gif)

## Choose an environment

Start with `Rustia::builder()`. It uses a fixed random seed and repeatable clocks.

- Use `RustiaBuilder::deterministic()` for simulations that need to replay the same way from the same inputs.
- Use `RustiaBuilder::live()` when a script needs the current time or a fresh random seed.

By default, scripts have no instruction or memory budget. To set one, call
`.fuel(100_000)` or `.memory_limit_bytes(2 * 1024 * 1024)` on the builder.
Fuel counts work done by the interpreter. The memory limit covers allocations
Rustia tracks for the script.

## Examples and API reference

From a checkout, run the embedding example or open the API reference in your browser.

```sh
git clone https://github.com/slowchop/rustia.git
cd rustia
cargo run --locked -p rustia --example embedding
cargo doc --locked -p rustia --no-deps --open
```

- [Read a Luau table into a Rust struct with Serde](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/embedding.rs)
- [Call methods on a Rust object from Luau](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/mutable_objects.rs)
- [Show the line that caused a script error](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/diagnostics.rs)
- [Run Rustia in a browser](https://github.com/slowchop/rustia/blob/main/demos/web-live/README.md)
- [Move an idle interpreter to another thread](https://github.com/slowchop/rustia/blob/main/design/75-embedded-api.md#transferable-application-facade-2026-11-17)

[Browse all Rust examples](https://github.com/slowchop/rustia/tree/main/crates/rustia/examples)

### Run Luau from the command line

From the same checkout, you can run a file or try one line of Luau.

```sh
cargo run --locked -p rustia-cli -- path/to/script.luau
cargo run --locked -p rustia-cli -- --eval "print(40 + 2)"
```

The second command prints `42`. For an interactive prompt, pass `--repl`.
Type `=40 + 2` to see the result and `:quit` to exit.

## How does it compare?

Rustia targets standalone Luau 0.709. Roblox services and Luau’s static type
checker are outside its scope. The [compatibility guide](https://github.com/slowchop/rustia/blob/main/conformance/COVERAGE.md)
lists the behavior tested against upstream Luau.

Rustia interprets Luau in Rust. [mlua](https://github.com/mlua-rs/mlua) embeds the upstream Lua
or Luau runtime. [Koto](https://koto.dev/) and [Rhai](https://rhai.rs/book/) each
provide their own scripting language.

### Speed: a measured snapshot

Upstream Luau via mlua was faster than Rustia on all five workloads in this run.
The chart includes upstream C++ Luau in interpreter and JIT modes alongside the Rust implementations.

![Time per workload for Rustia, luaur, ruau, C++ Luau in two modes, Koto, Rhai, and Roto. Lower is better.](https://github.com/slowchop/rustia/raw/6c1decf946af1a43e104d76fb8fb9ca9dfd11b30/crates/rustia/performance.svg)

Each language has its own version of each workload. Roto uses a list for sparse
lookups and a regular function for the closure test. The timings cover scripts already running. The [full results](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/RESULTS.md)
also show how long each engine took to get started.

<details>
<summary>See times for Rustia, mlua + Luau, Koto, and Rhai</summary>

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

[Raw measurements](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/current-results.json) · [Benchmark limitations](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/CLAIMS.md)

</details>

## Project status and source access

Rustia’s API may change before the first release. See [which platforms have been tested](https://github.com/slowchop/rustia/blob/main/design/96-browser-platform-acceptance.md)
and [what’s left before release](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md).

Maintainers track development in [GitHub Issues](https://github.com/slowchop/rustia/issues).
Policies for outside contributions and support are still undecided.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
