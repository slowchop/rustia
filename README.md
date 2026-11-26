# <img src="website/rustia-readme-mark.svg" alt="" width="26" height="26"> Rustia

<!-- project-copy: intro -->
Rustia is a pure-Rust interpreter for embedding standalone [Luau](https://luau.org/) in games and tools. Edit scripts without a rebuild and let players write mods.

Rustia grew out of a game by [Slowchop Studios](https://slowchop.com/) that needed to script character decisions and replay them on desktop and in a browser.
<!-- /project-copy: intro -->

**Version note.** [`0.1.0`](https://crates.io/crates/rustia/0.1.0) was an earlier
game-engine project I set aside. `0.2.0` is planned as the first release of this
Luau interpreter.

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

[Deterministic builder](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.deterministic)

</details>

<details>
<summary><strong>Pure Rust runtime</strong></summary>

One interpreter on desktop and web. Rustia’s runtime crates forbid `unsafe` Rust and build without a C++ toolchain.

[Platform comparison](https://rustia.dev/#luau-comparison)

</details>

<details>
<summary><strong>Sans-I/O core</strong></summary>

Embed Luau without relying on built-in platform services. Your app can supply clocks and randomness, decide where `print` output goes, and choose how to load scripts.

[Cargo features in Rustdoc](https://rustia.dev/docs/api/rustia/#cargo-features)

</details>

<details>
<summary><strong>Rust functions and objects</strong></summary>

Let scripts work with your app. Expose the functions and objects they need, and use Serde to read Luau tables into Rust structs.

[Embedding guide](https://rustia.dev/docs/api/rustia/#more-about-embedding)

</details>

<details>
<summary><strong>Script execution limits</strong></summary>

Keep runaway scripts under control. Set instruction and VM memory budgets so your app can handle a script that exceeds them.

[Budget example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hardened_decision.rs)

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
and add Rustia from Git. If you already have an app, run the last command from
its directory.

```sh
cargo new hello-rustia
cd hello-rustia
cargo add rustia --git https://github.com/slowchop/rustia --branch main
```

Expect API changes on `main` until Rustia reaches `1.0.0`.

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

- [`insert_global`](https://rustia.dev/docs/api/rustia/struct.Rustia.html#method.insert_global) gives Luau the starting score, and [`add_fn`](https://rustia.dev/docs/api/rustia/struct.Rustia.html#method.add_fn) lets it call the Rust function.
- [`source_name`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.source_name) gives the script a label that appears in error messages.
- [`eval`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.eval) gives you a value back in Rust. Use [`exec`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.exec) when you only want to run the script.

The example keeps the script in a string so it fits in one file. Read a `.luau` file at runtime if you want to edit it without rebuilding the Rust app.

From a Rustia checkout, run [the example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/first_script.rs) with `cargo run --locked -p rustia --example first_script`.
<!-- /project-copy: first-script -->

## Call script functions from Rust

The first script calls Rust from Luau. Here’s the other direction. Use
[`install`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.install)
to load a script that returns a table of exports. Look up `greet` with
[`Exports::function`](https://rustia.dev/docs/api/rustia/struct.Exports.html#method.function)
and [call it](https://rustia.dev/docs/api/rustia/struct.Function.html#method.call) from Rust.

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
until the edited version installs successfully. The [hot reload example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/hot_reload.rs)
shows a failed edit followed by a successful one.

## Choose an environment

Start with [`Rustia::builder()`](https://rustia.dev/docs/api/rustia/struct.Rustia.html#method.builder)
for everyday scripting. It starts with a fixed random seed and repeatable clocks.

- Use [`RustiaBuilder::deterministic()`](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.deterministic)
  to replay a simulation with the same inputs.
- Use [`RustiaBuilder::live()`](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.live)
  when a script needs the current time or a fresh random seed.

You can limit the work a script does with
[`fuel(100_000)`](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.fuel)
and limit the memory Rustia tracks for it with
[`memory_limit_bytes(2 * 1024 * 1024)`](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.memory_limit_bytes).
Both limits are off by default. After a memory-limit error, you can run another
script in the same interpreter. See [what happens when a script fails](https://rustia.dev/docs/api/rustia/#more-about-embedding).

## Cargo features

- **`platform-adapters`** is on by default. It provides system randomness and
  connects browser builds to JavaScript clocks, randomness, and console output.
  For WebAssembly without JavaScript imports, set `default-features = false`
  and give [`live()`](https://rustia.dev/docs/api/rustia/struct.RustiaBuilder.html#method.live)
  your own clocks and seed provider.
  [Example without JavaScript imports](https://github.com/slowchop/rustia/blob/main/demos/live-wasm/README.md)
- **`send`** makes `Rustia` and its live handles `Send`, so you can move
  them together to another thread. Calls still need `&mut Rustia`.
  See [thread ownership](https://rustia.dev/docs/api/rustia/struct.Rustia.html#threads)
  and the [runnable example](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/transferable.rs).

  From a Rustia checkout, run `cargo run --locked -p rustia --features send --example transferable`.

## Examples and API reference

Read the [API reference](https://rustia.dev/docs/api/) online, or run the embedding example and build Rustdoc from a checkout.

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
- [Move an idle interpreter and a retained function to another thread](https://github.com/slowchop/rustia/blob/main/crates/rustia/examples/transferable.rs)

[Browse all Rust examples](https://github.com/slowchop/rustia/tree/main/crates/rustia/examples)

### Run Luau from the command line

Run this command from the checkout to try one line of Luau.

```sh
cargo run --locked --bin rustia -- --eval "print(40 + 2)"
```

The command prints `42`. To run a file, pass its path after `--`. For an interactive prompt, pass `--repl`.
Type `=40 + 2` to see the result and `:quit` to exit.

<a id="how-does-it-compare"></a>

## Rustia and other Luau projects

<a id="compare-luau-projects"></a>

[Compare Luau projects](https://rustia.dev/#luau-comparison) for platform support, hot reload, and replay. The [comparison notes](https://rustia.dev/docs/api/rustia/#compare-luau-projects) include the sources. See [tested language features](https://github.com/slowchop/rustia/blob/main/conformance/COVERAGE.md) for Rustia’s Luau compatibility.

[Benchmark methodology](https://github.com/slowchop/rustia/blob/da1b8e546d4108d657d875b60ad28dffb354a974/bench/CLAIMS.md)

## Project status and source access

See [which platforms have been tested](https://github.com/slowchop/rustia/blob/main/design/96-browser-platform-acceptance.md)
and [what’s left before release](https://github.com/slowchop/rustia/blob/main/design/90-release-readiness.md).

Bug reports and focused pull requests are welcome. See the
[contribution guide](https://github.com/slowchop/rustia/blob/main/CONTRIBUTING.md#contributing)
and [private security reporting instructions](https://github.com/slowchop/rustia/security/policy).

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
