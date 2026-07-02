-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers mixed named and computed table fields.
local t = {data = 4, [1 + 2] = 5}
return t.data + t[3]
