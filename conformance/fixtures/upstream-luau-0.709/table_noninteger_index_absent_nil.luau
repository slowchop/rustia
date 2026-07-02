-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers reading an absent non-integer numeric table key.
local t = {6, 9, 7}
return t[4.5]
