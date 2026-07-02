-- Derived from Luau 0.709 tests/conformance/sort.luau table.sort argument checks.
assert(pcall(table.sort, {}, 42) == false)
assert(pcall(table.sort, {}, {}) == false)
return "OK"
