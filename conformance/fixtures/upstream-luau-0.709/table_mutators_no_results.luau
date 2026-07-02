-- Derived from Luau 0.709 tests/conformance/tables.luau table.insert return-value check.
local t = {}
assert(select("#", table.insert(t, 42)) == 0)
assert(t[1] == 42)

return "OK"
