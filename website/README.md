# Rustia website

The homepage and playground are static. Rustia and the Luau analyzer run as Wasm
in browser workers. Drafts stay in browser storage. Console projects only
resolve modules from their own files. Sprite projects expose drawing and
plain-text output to the page.

## Garden source

- [wandering-flowers.luau](examples/garden-studies/wandering-flowers.luau) is the script in the homepage editor.
- [flowerbed.luau](flowerbed.luau) keeps the flowers across edits, and [drawing.luau](drawing.luau) defines the script's drawing API.
- [garden.rs](garden.rs), [worker.mjs](worker.mjs), and [garden-player.mjs](garden-player.mjs) connect the script to the browser animation.
- [draw-garden.mjs](draw-garden.mjs) draws the sprites into the page.

## Build and check

From the repository root:

```sh
bash website/build.sh
node website/check-garden-api.mjs
node website/check-garden-setup.mjs
python3 -m http.server --directory website/dist 8080
```

The build uses this checkout of Rustia, the pinned Rust toolchain, and the
version of wasm-bindgen in Cargo.lock. Set an absolute CARGO_TARGET_DIR to place
compiler output on another volume. The browser analyzer and editor bundle are
vendored with their license notices. The generated runtime and publish directory
are ignored by Git.

## Cloudflare Workers Static Assets

Connect the Rustia repository in Workers Builds, select `main` as the
production branch, and leave the root directory at the repository root.
Use `bash website/build.sh` for the build command. The script installs
Rustup if the build image lacks it, then selects the pinned Rust toolchain.

Set the deploy command to the following for a Worker named `rustia`:

```sh
npx wrangler@4.73.0 deploy --name rustia --assets ./website/dist --compatibility-date 2026-11-23
```

Verify the `workers.dev` deployment before adding the custom domain. Cloudflare
serves the staged files as static assets, with response policies from
`_headers`. The site's data stays local to visitors' browsers.
