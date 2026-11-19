#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
exec sh "$root/tools/browser/render-demo-gif.sh" \
    "${1:-$root/target/demo/rustia-hot-reload.mp4}" "$root/demos/web-live/hot-reload.gif"
