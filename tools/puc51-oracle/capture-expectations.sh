#!/bin/sh
# Regenerate MANIFEST.tsv expectation fields for the puc51-compat fixtures from
# the pinned PUC 5.1.5 oracle (third_party/lua-src @ 4518bd0, stock 5.1.5).
# Prints one "<fixture-file>\t<expect-field>" row per fixture to stdout.
#
# Oracle hygiene: fixtures return strings serialized in-guest via tostring /
# concat / table.concat / string.format("%s") — no number is ever formatted
# host-side. LC_ALL=C pins the C locale decimal point.
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ORACLE="$ROOT/tools/puc51-oracle/build/puc51-oracle"

if [ ! -x "$ORACLE" ]; then
    sh "$ROOT/tools/puc51-oracle/build.sh" >&2
fi

export LC_ALL=C

for f in "$ROOT"/conformance/fixtures/puc51-compat/*.luau; do
    out="$("$ORACLE" "$f")"
    if printf '%s' "$out" | grep -q '	'; then
        echo "$(basename "$f"): payload contains a tab; use string_hex" >&2
        exit 1
    fi
    printf '%s\tstring:%s\n' "$(basename "$f")" "$out"
done
