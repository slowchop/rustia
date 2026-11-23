#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

toolchain=1.95.0
bindgen=0.2.125
target=wasm32-unknown-unknown
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"

# Cloudflare's build image can start without Rust installed.
if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --no-modify-path
  export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi

rustup toolchain install "$toolchain" --profile minimal --target "$target"
if ! command -v wasm-bindgen >/dev/null || [[ "$(wasm-bindgen --version)" != "wasm-bindgen $bindgen" ]]; then
  tools="$CARGO_TARGET_DIR/site-tools"
  if [[ ! -x "$tools/bin/wasm-bindgen" ]]; then
    cargo +"$toolchain" install wasm-bindgen-cli --version "=$bindgen" --locked --root "$tools"
  fi
  export PATH="$tools/bin:$PATH"
fi
cargo +"$toolchain" build --locked --target "$target" --release

mkdir -p dist/pkg dist/analysis dist/playground/race dist/examples/garden dist/examples/garden-studies
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
  "$CARGO_TARGET_DIR/$target/release/rustia_garden.wasm"
cp index.html _headers LICENSE-* garden.rs bridge.rs codemirror.js garden.css sprite-names.txt \
  sprite-defs.html garden-fragment.html *.luau dist/
for script in *.mjs; do
  case "$script" in
    check-*) ;;
    *) cp "$script" dist/ ;;
  esac
done
cp analysis/analyzer.mjs analysis/analyzer.wasm analysis/LICENSE.txt analysis/lua_LICENSE.txt dist/analysis/
cp -R analysis/licenses dist/analysis/
cp playground/{index.html,*.mjs,*.luau,playground.css} dist/playground/
cp playground/race/index.html dist/playground/race/
cp examples/garden/index.html dist/examples/garden/
cp examples/garden-studies/{index.html,*.luau} dist/examples/garden-studies/
