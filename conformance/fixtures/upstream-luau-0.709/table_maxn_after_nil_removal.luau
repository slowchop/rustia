-- Derived from Luau 0.709 tests/conformance/tables.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local a = {[10] = 1, [20] = 2}
a[20] = nil
return table.maxn(a)
