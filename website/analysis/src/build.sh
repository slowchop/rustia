#!/bin/sh
set -eu
# Use an activated Emscripten SDK and an external build directory.
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
luau=${1:?usage: build.sh LUAU_CHECKOUT BUILD_DIR OUTPUT_DIR}
build=${2:?missing external build directory}
out=${3:?missing output directory}
test "$(git -C "$luau" rev-parse HEAD)" = c0e346edd89066b44dca174c9f54ce84c746a540
emcc --version | head -1 | grep -F '4.0.23'
emcmake cmake -S "$here" -B "$build" -DLUAU_SOURCE="$luau" -DCMAKE_BUILD_TYPE=Release
cmake --build "$build" --target analyzer --parallel "${BUILD_JOBS:-4}"
mkdir -p "$out"
cp "$build/analyzer.mjs" "$build/analyzer.wasm" "$luau/LICENSE.txt" "$out/"
cp "$luau/lua_LICENSE.txt" "$out/"
emroot=$(dirname "$(command -v emcc)")
mkdir -p "$out/licenses"
cp "$emroot/LICENSE" "$out/licenses/Emscripten.txt"
cp "$emroot/system/lib/libc/musl/COPYRIGHT" "$out/licenses/musl.txt"
for lib in libcxx libcxxabi compiler-rt; do
    cp "$emroot/system/lib/$lib/LICENSE.TXT" "$out/licenses/$lib.txt"
done
