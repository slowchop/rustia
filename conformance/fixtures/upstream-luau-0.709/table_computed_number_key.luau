-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers computed numeric table literal keys read through bracket indexing.
local t = {[1 + 2] = 4}
return t[3]
