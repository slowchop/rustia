-- Derived from Luau 0.709 tests/conformance/move.luau move-backward checks.
local a = table.move({10, 20, 30}, 2, 3, 1)
return table.concat(a, ",")
