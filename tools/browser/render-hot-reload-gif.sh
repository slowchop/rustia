#!/bin/sh
# Convert the verified headless recording into the animation embedded in the README.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
input=${1:-"$root/target/demo/rustia-hot-reload.mp4"}
output="$root/demos/web-live/hot-reload.gif"
staging=$(mktemp -d "${output}.XXXXXX")
trap 'rm -rf "$staging"' EXIT
trap 'exit 1' HUP INT TERM

ffmpeg -hide_banner -loglevel error -nostdin -i "$input" \
    -filter_complex 'fps=10,split[a][b];[a]palettegen[p];[b][p]paletteuse=dither=none:diff_mode=rectangle' \
    -loop 0 "$staging/demo.gif"
mv "$staging/demo.gif" "$output"
