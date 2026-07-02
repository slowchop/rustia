-- Derived from Luau 0.709 tests/conformance/move.luau empty move checks.
local a = {}
table.move({10, 20, 30}, 1, 0, 3, a)
return table.getn(a)
