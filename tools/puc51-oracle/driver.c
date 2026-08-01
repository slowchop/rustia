#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"

/* Offline PUC 5.1.5 oracle driver: runs a Lua file and lets guest `print`
 * write to stdout. Used only by the fixture generator — never built by CI.
 */
int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <script.lua>\n", argv[0]);
        return 2;
    }
    lua_State *L = luaL_newstate();
    if (L == NULL) {
        fprintf(stderr, "failed to create lua state\n");
        return 1;
    }
    luaL_openlibs(L);
    if (luaL_dofile(L, argv[1]) != 0) {
        fprintf(stderr, "error: %s\n", lua_tostring(L, -1));
        lua_close(L);
        return 1;
    }
    lua_close(L);
    return 0;
}
