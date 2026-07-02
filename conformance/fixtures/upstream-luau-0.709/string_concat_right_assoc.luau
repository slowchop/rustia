-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers right-associative string concatenation: a .. '2' .. '3'.
local a = '1'
a = a .. '2' .. '3'
return a
