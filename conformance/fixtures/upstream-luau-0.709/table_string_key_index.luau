-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers reading a string table key through bracket indexing.
local t = {6, 9, 7, a = 11}
return t["a"]
