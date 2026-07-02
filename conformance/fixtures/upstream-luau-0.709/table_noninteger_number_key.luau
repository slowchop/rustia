-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers reading a non-integer numeric table key assigned in a literal.
local t = {6, 9, 7, [4.5] = 11}
return t[4.5]
