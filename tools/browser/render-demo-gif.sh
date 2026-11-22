#!/bin/sh
# Convert a verified headless recording into a looping GIF.
set -eu

input=${1:?expected an input MP4 and an output GIF}
output=${2:?expected an output GIF}
staging=$(mktemp -d "${output}.XXXXXX")
trap 'rm -rf "$staging"' EXIT
trap 'exit 1' HUP INT TERM

ffmpeg -hide_banner -loglevel error -nostdin -i "$input" \
    -filter_complex 'fps=10,split[a][b];[a]palettegen[p];[b][p]paletteuse=dither=none:diff_mode=rectangle' \
    -loop 0 "$staging/demo.gif"
mv "$staging/demo.gif" "$output"
