# Embed Rustia in a browser with JavaScript services

This standalone application keeps Rustia's default `web-live` feature and uses
`wasm-bindgen` to connect WebAssembly to JavaScript. It runs a short Luau script,
prints `Hello from Luau` in the browser console, and displays its returned number
as `Result: 42`. It does not measure performance.

## Build and run

Start in the repository root. You need Rustup and Cargo, Python 3, and a browser
that supports WebAssembly and JavaScript modules. Install the pinned Rust target
and a `wasm-bindgen` CLI matching this example's dependency:

```sh
rustup toolchain install 1.95.0 --profile minimal --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
```

Build the application, then generate the JavaScript loader and its Wasm artifact:

```sh
cargo build --locked --manifest-path demos/web-live/Cargo.toml --target wasm32-unknown-unknown --release --target-dir demos/web-live/target
wasm-bindgen --target web --out-dir demos/web-live/pkg --out-name rustia_example demos/web-live/target/wasm32-unknown-unknown/release/rustia_web_live_example.wasm
```

Serve the directory over HTTP:

```sh
python3 -m http.server 8788 --bind 127.0.0.1 --directory demos/web-live
```

Open [the local example](http://127.0.0.1:8788/). Wait for `Ready`, then click
**Run example**. The page should show `Result: 42`; open the browser's developer
console to see `Hello from Luau`. Stop the server with Ctrl-C.

Generated files live in `pkg/` and `target/`; they are not source files. Serve
the directory rather than opening `index.html` through a `file:` URL, so the
module loader can fetch the Wasm artifact.

## How the application connects

[Cargo.toml](./Cargo.toml) leaves Rustia's default features enabled and pins
`wasm-bindgen` to `0.2.126`. [src/lib.rs](./src/lib.rs) exports `run_example` to
JavaScript. It explicitly selects `RustiaBuilder::live()`, sets 10,000 instruction
fuel and a 2 MiB accounted-memory limit, then evaluates
`print('Hello from Luau'); return 6 * 7` as an `i32` with the source name
`browser-example.luau`.

[index.html](./index.html) loads the generated module. Initialize its imports
before calling the Rust export:

```js
import init, { run_example } from "./pkg/rustia_example.js";

await init();
const result = document.querySelector("#result");
try {
  result.textContent = `Result: ${run_example()}`;
} catch (error) {
  result.textContent = `Script failed: ${String(error)}`;
}
```

The actual page keeps the button disabled until initialization completes and
reports initialization failures separately. Rust converts build or script errors
to diagnostic strings before returning them as JavaScript exceptions. The page
uses `textContent` and `String(error)` to show those messages as text.

## Feature and preset are separate choices

The `web-live` Cargo feature makes JavaScript adapters available. Enabling it
does not select the live environment. `RustiaBuilder::standard()` still uses a
fixed random seed and repeatable clocks; on browser Wasm, `print` is silent
until the application supplies a sink.

This application explicitly uses `RustiaBuilder::live()`. With `web-live`, that
preset connects output to `console`, wall time to `Date`, monotonic time to
`performance`, and the random seed to host entropy through `crypto`. It also
enables runtime compilation through `loadstring`. Choose it when the application
needs those services and permits results that need not repeat.

For direct `WebAssembly.instantiate` with no JavaScript imports, see the
[bare-Wasm demo](../live-wasm/README.md). It disables `web-live` and uses the
standard environment. This recipe is one integration example; browser execution
evidence and the remaining Safari/mobile release gates are recorded separately
in [platform acceptance](../../design/96-browser-platform-acceptance.md). A headless
WebKit check alone does not establish Safari or mobile acceptance.
