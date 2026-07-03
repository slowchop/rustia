-- Derived from Luau 0.709 VM/src/lbaselib.cpp explicit-base tonumber path.
local wrapped = 2 ^ 64

assert(tonumber("-1", 16) == wrapped)
assert(tonumber("-10", 16) == wrapped)
assert(tonumber("  -f  ", 16) == wrapped)
assert(tonumber("+f", 16) == 15)

return "OK"
