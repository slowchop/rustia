-- Derived from Luau 0.709 tests/conformance/tables.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- The upstream table is reduced to its final observable shape: only key 1000 remains.
local a = {}
a[1000] = 1000
return next(a, nil)
