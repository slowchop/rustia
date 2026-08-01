#!/bin/sh
# Build the offline PUC 5.1.5 oracle from the pinned submodule sources.
# Output: tools/puc51-oracle/build/puc51-oracle (gitignored).
#
# The submodule (third_party/lua-src @ v550.1.1) is verified against the
# official lua-5.1.5.tar.gz (MD5 2e115fe26e435e33b0d5c022e4490567): it adds
# WASI guards (inert here) and one ldo.c stack-check hunk, which this script
# reverts before compiling so the oracle is stock 5.1.5.
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$ROOT/third_party/lua-src/lua-5.1.5"
BUILD="$ROOT/tools/puc51-oracle/build"

if [ ! -f "$SRC/ltable.c" ]; then
    echo "submodule missing: run git submodule update --init third_party/lua-src" >&2
    exit 1
fi

rm -rf "$BUILD"
mkdir -p "$BUILD/src"
cp "$SRC"/*.c "$SRC"/*.h "$BUILD/src/"

# Revert the lua-src-rs ldo.c patch to stock 5.1.5.
sed -i.bak 's/luaD_checkstack(L, p->maxstacksize + p->numparams);/luaD_checkstack(L, p->maxstacksize);/' "$BUILD/src/ldo.c" && rm "$BUILD/src/ldo.c.bak"
grep -q 'luaD_checkstack(L, p->maxstacksize);' "$BUILD/src/ldo.c" || {
    echo "failed to revert ldo.c stack-check hunk" >&2
    exit 1
}

cp "$ROOT/tools/puc51-oracle/driver.c" "$BUILD/driver.c"

# Stock luaconf.h already defines LUA_COMPAT_MOD/LUA_COMPAT_GFIND.
CC="${CC:-cc}"
CFLAGS="-O2 -DLUA_USE_LINUX"
"$CC" $CFLAGS -I"$BUILD/src" -o "$BUILD/puc51-oracle" "$BUILD"/src/*.c "$BUILD/driver.c" -lm
echo "built $BUILD/puc51-oracle"
