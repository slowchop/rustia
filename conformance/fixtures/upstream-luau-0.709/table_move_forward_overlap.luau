-- Derived from Luau 0.709 tests/conformance/move.luau move-forward overlap checks.
local a = table.move({10, 20, 30}, 1, 3, 2)
return table.concat(a, ",")
