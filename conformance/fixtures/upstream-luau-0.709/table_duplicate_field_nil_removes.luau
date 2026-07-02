-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers duplicate named table fields where the final nil removes the field.
local t = {data = 4, data = nil, data = 42, data = nil}
return t.data
