-- Derived from Luau 0.709 VM/src/lbaselib.cpp explicit-base tonumber path.
assert(tonumber("0x10", 16) == 16)
assert(tonumber("0Xff", 16) == 255)
assert(tonumber("  0x10  ", 16) == 16)
assert(tonumber("0x", 16) == nil)
assert(tonumber("0xg", 16) == nil)

return "OK"
