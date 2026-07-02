-- Derived from Luau 0.709 tests/conformance/tables.luau global-cleanup string.find check.
assert(string.find("ABC", "^[%u_]") == 1)
assert(string.find("_x", "^[%u_]") == 1)
assert(string.find("abc", "^[%u_]") == nil)
assert(string.find("ABC", "^[%u_]", 2) == nil)

return "OK"
