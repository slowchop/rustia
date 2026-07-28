# Rustia Live Wasm Demo

This is a standalone browser demo for the current P1 Rustia engine. It is intentionally outside
the root Cargo workspace membership so normal workspace gates do not build browser artifacts.

Build the wasm runner:

```sh
cargo build --locked --manifest-path demos/live-wasm/wasm-runner/Cargo.toml --target wasm32-unknown-unknown --release
```

Serve the static page:

```sh
python3 -m http.server 8787 --directory demos/live-wasm
```

Then open <http://127.0.0.1:8787/>.

The runner deliberately disables Rustia's default `web-live` feature because this page uses
`WebAssembly.instantiate(..., {})` directly. Applications using `RustiaBuilder::live()` in a
browser should keep the default feature and use the wasm-bindgen toolchain instead.
