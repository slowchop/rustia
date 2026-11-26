# Rustia bare-Wasm browser demo

This standalone browser demo evaluates Luau source with Rustia's standard
environment. It instantiates raw Wasm with an empty import object and disables
Rustia's default `platform-adapters` feature. The directory name `live-wasm` does not mean
that it selects `RustiaBuilder::live()` or JavaScript host services. The runner
disables stdout, so the page displays returned values rather than script `print`
output.

For a complete `wasm-bindgen` setup using the default JavaScript adapters and
`RustiaBuilder::live()`, follow [the JavaScript-services recipe](../web-live/README.md).

The demo remains outside the root Cargo workspace. Run these commands from the
repository root.

Build the wasm runner:

```sh
cargo build --locked --manifest-path demos/live-wasm/wasm-runner/Cargo.toml --target wasm32-unknown-unknown --release --target-dir demos/live-wasm/wasm-runner/target
```

Serve the static page:

```sh
python3 -m http.server 8787 --bind 127.0.0.1 --directory demos/live-wasm
```

Then open [the local demo](http://127.0.0.1:8787/). Its source is in
[index.html](./index.html) and [the runner](./wasm-runner/src/lib.rs).
