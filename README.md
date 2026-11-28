# <img src="website/rustia-readme-mark.svg" alt="" width="26" height="26"> Rustia

[![crates.io](https://img.shields.io/crates/v/rustia)](https://crates.io/crates/rustia)
[![docs.rs](https://img.shields.io/docsrs/rustia/0.2.0)](https://docs.rs/rustia/0.2.0/rustia/)
[![CI on main](https://github.com/slowchop/rustia/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/slowchop/rustia/actions/workflows/ci.yml?query=branch%3Amain)

<!-- project-copy: intro -->
Rustia is a pure-Rust interpreter for embedding standalone [Luau](https://luau.org/) in games and tools. Edit scripts without a rebuild and let players write mods.

We built Rustia at [Slowchop Studios](https://slowchop.com/) for a [Bevy](https://bevy.org/) game. We wanted to tweak gameplay without waiting for Rust to recompile, run scripts quickly and safely, and get deterministic results for multiplayer and replays on desktop and in the browser.
<!-- /project-copy: intro -->

**Version note.** [`0.1.0`](https://crates.io/crates/rustia/0.1.0) was an earlier
game-engine project I set aside. [`0.2.0`](https://crates.io/crates/rustia/0.2.0) is the first
release of this Luau interpreter.

[Run your first script](#try-it) or [try the browser playground](https://rustia.dev/playground/).

## Features

<!-- project-copy: features -->
<details>
<summary><strong>Hot reload</strong></summary>

Edit Luau without a rebuild. Your app keeps running and can carry state across edits, like the flowers’ positions in the garden. If a reload fails, Rustia can restore the previous script state.

[Try the garden](https://rustia.dev/)

</details>

<details>
<summary><strong>Deterministic execution</strong></summary>

Replay a run. In deterministic mode, the same script, starting state, seed, and ordered inputs produce matching results on native and WebAssembly. That helps reproduce bugs and keep lockstep multiplayer clients in sync.

[Deterministic builder](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.deterministic)

</details>

<details>
<summary><strong>Parallel script execution</strong></summary>

Run independent Luau VMs on Rust worker threads. Each VM has its own script state, and your app schedules the work.

[Thread ownership in Rustdoc](https://docs.rs/rustia/0.2.0/rustia/struct.Rustia.html#threads)

</details>

<details>
<summary><strong>Pure Rust runtime</strong></summary>

One interpreter on desktop and web. Rustia’s runtime crates forbid `unsafe` Rust and build without a C++ toolchain.

[Platform comparison](https://rustia.dev/#luau-comparison)

</details>

<details>
<summary><strong>Sans-I/O core</strong></summary>

Embed Luau without relying on built-in platform services. Your app can supply clocks and randomness, decide where `print` output goes, and choose how to load scripts.

[Cargo features in Rustdoc](https://docs.rs/rustia/0.2.0/rustia/#cargo-features)

</details>

<details>
<summary><strong>Rust functions and objects</strong></summary>

Let scripts work with your app. Expose the functions and objects they need, and use Serde to read Luau tables into Rust structs.

[Embedding guide](https://docs.rs/rustia/0.2.0/rustia/#more-about-embedding)

</details>

<details>
<summary><strong>Script execution limits</strong></summary>

Keep runaway scripts under control. Set instruction and VM memory budgets so your app can handle a script that exceeds them.

[Budget example](https://docs.rs/crate/rustia/0.2.0/source/examples/hardened_decision.rs)

</details>
<!-- /project-copy: features -->

## In development

<!-- project-copy: upcoming -->
<details>
<summary><strong>Time-travel debugging</strong></summary>

Step back through a recorded Luau run, inspect earlier values, and find the line that last changed a variable. [Follow the debugger work](https://github.com/slowchop/rustia/issues/71).

</details>

<details>
<summary><strong>A Luau layer for Bevy</strong></summary>

We’re building a Luau layer on top of [Bevy](https://bevy.org/) so you can change gameplay while the game runs, without rebuilding the Rust app.

</details>

<details>
<summary><strong>JIT and AOT research</strong></summary>

We’re exploring just-in-time (JIT) compilation in the background during development and ahead-of-time (AOT) compilation for bundled scripts. Today Rustia interprets Luau bytecode. [Follow the compilation work](https://github.com/slowchop/rustia/issues/89).

</details>

<details>
<summary><strong>Synth in Luau</strong></summary>

We’re also having fun with a Luau synth in the browser. Write and tweak sounds live, record MIDI performances, and record audio from instruments.

</details>
<!-- /project-copy: upcoming -->

## Try it

You’ll need [Rust](https://rustup.rs/) 1.95 or newer. Create a small Cargo app
and add Rustia from crates.io. If you already have an app, run the last command from
its directory.

```sh
cargo new hello-rustia
cd hello-rustia
cargo add rustia@0.2.0
```

The API can change before Rustia reaches `1.0.0`.

### A first script

<!-- project-copy: first-script -->
Start with a score of 21 and a Rust function called `double`. The Luau script calls `double(base_score)` and sends 42 back to Rust.

In the Cargo project you [set up with Rustia](https://github.com/slowchop/rustia#try-it), replace `src/main.rs` with this code and run `cargo run`.

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

You should see `The score is 42`.

- [`insert_global`](https://docs.rs/rustia/0.2.0/rustia/struct.Rustia.html#method.insert_global) gives Luau the starting score, and [`add_fn`](https://docs.rs/rustia/0.2.0/rustia/struct.Rustia.html#method.add_fn) lets it call the Rust function.
- [`source_name`](https://docs.rs/rustia/0.2.0/rustia/struct.Chunk.html#method.source_name) gives the script a label that appears in error messages.
- [`eval`](https://docs.rs/rustia/0.2.0/rustia/struct.Chunk.html#method.eval) gives you a value back in Rust. Use [`exec`](https://docs.rs/rustia/0.2.0/rustia/struct.Chunk.html#method.exec) when you only want to run the script.

The example keeps the script in a string so it fits in one file. Read a `.luau` file at runtime if you want to edit it without rebuilding the Rust app.

From a Rustia checkout, run [the example](https://docs.rs/crate/rustia/0.2.0/source/examples/first_script.rs) with `cargo run --locked -p rustia --example first_script`.
<!-- /project-copy: first-script -->

## Call script functions from Rust

The first script calls Rust from Luau. Here’s the other direction. Use
[`install`](https://docs.rs/rustia/0.2.0/rustia/struct.Chunk.html#method.install)
to load a script that returns a table of exports. Look up `greet` with
[`Exports::function`](https://docs.rs/rustia/0.2.0/rustia/struct.Exports.html#method.function)
and [call it](https://docs.rs/rustia/0.2.0/rustia/struct.Function.html#method.call) from Rust.

<details>
<summary>Show the Rust program</summary>

Replace `src/main.rs` with this example and run `cargo run` again. The program prints `Hello, Ada`.

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

</details>

If you’re reloading a script while your app runs, keep calling the old functions
until the edited version installs successfully. The [hot reload example](https://docs.rs/crate/rustia/0.2.0/source/examples/hot_reload.rs)
shows a failed edit followed by a successful one.

## Choose an environment

Start with [`Rustia::builder()`](https://docs.rs/rustia/0.2.0/rustia/struct.Rustia.html#method.builder)
for everyday scripting. It starts with a fixed random seed and repeatable clocks.

- Use [`RustiaBuilder::deterministic()`](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.deterministic)
  to replay a simulation with the same inputs.
- Use [`RustiaBuilder::live()`](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.live)
  when a script needs the current time or a fresh random seed.

You can limit the work a script does with
[`fuel(100_000)`](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.fuel)
and limit the memory Rustia tracks for it with
[`memory_limit_bytes(2 * 1024 * 1024)`](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.memory_limit_bytes).
Both limits are off by default. After a memory-limit error, you can run another
script in the same interpreter. See [what happens when a script fails](https://docs.rs/rustia/0.2.0/rustia/#more-about-embedding).

## Cargo features

- **`platform-adapters`** is on by default. It provides system randomness and
  connects browser builds to JavaScript clocks, randomness, and console output.
  For WebAssembly without JavaScript imports, set `default-features = false`
  and give [`live()`](https://docs.rs/rustia/0.2.0/rustia/struct.RustiaBuilder.html#method.live)
  your own clocks and seed provider.
  [Example without JavaScript imports](https://github.com/slowchop/rustia/blob/main/demos/live-wasm/README.md)
- **`send`** makes `Rustia` and its live handles `Send`, so you can move
  them together to another thread. Calls still need `&mut Rustia`.
  See [thread ownership](https://docs.rs/rustia/0.2.0/rustia/struct.Rustia.html#threads)
  and the [runnable example](https://docs.rs/crate/rustia/0.2.0/source/examples/transferable.rs).

  From a Rustia checkout, run `cargo run --locked -p rustia --features send --example transferable`.

## Examples and API reference

Read the [API reference](https://docs.rs/rustia/0.2.0/rustia/) online, or run the embedding example and build Rustdoc from a checkout.

```sh
git clone --branch release/0.2 https://github.com/slowchop/rustia.git
cd rustia
git checkout --detach c89abc121b5233d29353846217d2db95b93e8a34
cargo run --locked -p rustia --example embedding
cargo doc --locked -p rustia --no-deps --open
```

- [Read a Luau table into a Rust struct with Serde](https://docs.rs/crate/rustia/0.2.0/source/examples/embedding.rs)
- [Call methods on a Rust object from Luau](https://docs.rs/crate/rustia/0.2.0/source/examples/mutable_objects.rs)
- [Show the line that caused a script error](https://docs.rs/crate/rustia/0.2.0/source/examples/diagnostics.rs)
- [Run Rustia in a browser](https://github.com/slowchop/rustia/blob/main/demos/web-live/README.md)
- [Move an idle interpreter and a retained function to another thread](https://docs.rs/crate/rustia/0.2.0/source/examples/transferable.rs)

[Browse all Rust examples](https://docs.rs/crate/rustia/0.2.0/source/examples/)

### Run Luau from the command line

Install the [Rustia CLI](https://crates.io/crates/rustia-cli/0.2.0), then try one line of Luau.

```sh
cargo install rustia-cli --version 0.2.0 --locked
rustia --eval "print(40 + 2)"
```

The command prints `42`. To run a file, pass its path to `rustia`. For an interactive prompt, pass `--repl`.
Type `=40 + 2` to see the result and `:quit` to exit.

<a id="how-does-it-compare"></a>

## Rustia and other Luau projects

<a id="compare-luau-projects"></a>

[Compare Luau projects](https://rustia.dev/#luau-comparison) for platform support, hot reload, and replay. The [comparison notes](https://docs.rs/rustia/0.2.0/rustia/#compare-luau-projects) include the sources. See [tested language features](https://github.com/slowchop/rustia/blob/main/conformance/COVERAGE.md) for Rustia’s Luau compatibility.

[Benchmark methodology](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/CLAIMS.md)

## Project status and source access

See [which platforms have been tested](https://github.com/slowchop/rustia/blob/main/design/96-browser-platform-acceptance.md)
and [what’s left before release](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md).

Bug reports and focused pull requests are welcome. See the
[contribution guide](https://github.com/slowchop/rustia/blob/main/CONTRIBUTING.md#contributing)
and [private security reporting instructions](https://github.com/slowchop/rustia/security/policy).

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
