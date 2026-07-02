-- Derived from Luau 0.709 tests/conformance/tables.luau NaN-position table.insert check.
local a = { 1, 2, 3 }
table.insert(a, 0 / 0, 42)
return 1
