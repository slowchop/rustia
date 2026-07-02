-- Derived from Luau 0.709 tests/conformance/tables.luau table.move boundary invariant check.
local t1 = {1, 2, 3, 4, 5}
local t2 = {[6] = 6}

table.move(t1, 1, 5, 1, t2)
return #t2
