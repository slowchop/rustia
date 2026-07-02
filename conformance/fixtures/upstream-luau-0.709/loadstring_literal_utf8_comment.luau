-- Derived from Luau 0.709 tests/conformance/strings.luau loadstring literal check.
local chunk = loadstring("return 1\n--comentário sem EOL no final")

assert(type(chunk) == "function")
assert(chunk() == 1)

return "OK"
