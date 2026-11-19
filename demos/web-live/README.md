# Embed Rustia in a browser with JavaScript services

[Compare Rustia with other Luau projects](./compare.html) for features, platform support, and measured performance.

This standalone application keeps Rustia's default `web-live` feature and uses
`wasm-bindgen` to connect WebAssembly to JavaScript. It runs a short Luau script,
prints `Hello from Luau` in the browser console, and displays its returned number
as `Result: 42`. It does not measure performance.

## Build and run

For the race recording, see [Record the race](#record-the-race).

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

## Record the race

The recorder runs [the same Luau script](./race.luau) in a native executable and
in WebAssembly inside headless WebKit. Each seed starts a fresh deterministic
VM. The recorder checks every racer's distance at every tick, then draws the
two runs side by side. Seeds 7 and 11 are recorded in one 15-second clip.

Use the pinned Rust target and wasm-bindgen CLI from the build instructions,
Node 22, and FFmpeg with `libx264` and `ffprobe`. From the repository root,
install the browser dependencies once:

```sh
npm ci --prefix tools/browser --ignore-scripts
tools/browser/node_modules/.bin/playwright install webkit
```

Run `just record-race`. Open `target/demo/rustia-replay.mp4` to watch it.
For a different destination, run `node tools/browser/record-race.cjs path/to/race.mp4`.
Set `CARGO_TARGET_DIR` to choose the build volume. To change the race, edit
`race.luau` and record again. The existing MP4 is replaced after the new
recording passes its trace and frame checks.

Run `just readme-race` to record both races and export the [README animation](./race.gif).
After committing a new GIF, use its commit SHA in the race image URL in the project README.

The WebKit CI job records both clips automatically and retains them in the
`rustia-replay` artifact for seven days. These clips show the headless WebKit engine. Physical iPad Safari
execution has its own [platform evidence](../../design/96-browser-platform-acceptance.md).

## Edit a running race

After the browser build and serve steps above, open
[the local race editor](http://127.0.0.1:8788/hot-reload.html).
Place the caret after the `2` in `return x + 2` while the racer is moving.
Press Backspace and type `2` again. After a few ticks, press Backspace again
and pause. The editor shows Luau's error while the last good rule keeps the racer
moving two metres per tick. Type `4` to change its speed to four metres per
tick from its current position.

Run `just record-hot-reload` to record the same sequence in headless WebKit.
The caret blinks at the insertion point, and a keycap at the bottom shows each
keypress. The recorder checks the racer's position at every tick and saves a
12-second, 1280×720 clip in `target/demo/rustia-hot-reload.mp4`. To choose a different path, use
`node tools/browser/record-race.cjs --hot-reload path/to/edits.mp4`.

Run `just readme-hot-reload` to record the demo and refresh the [README animation](./hot-reload.gif).
To use an existing recording, run `sh tools/browser/render-hot-reload-gif.sh path/to/edits.mp4`
from the repository root.
After committing a new GIF, update the image URL in the project README to use
that commit's full SHA. This keeps each README linked to its own recording.
