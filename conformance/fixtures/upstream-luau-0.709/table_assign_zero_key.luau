-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers assigning numeric key zero as a table hash key.
local t = {6, 9, 7}
t[0] = 5
return t[0]
