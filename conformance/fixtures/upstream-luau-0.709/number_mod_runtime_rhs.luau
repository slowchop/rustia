-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Keeps the RHS runtime-visible instead of folding the modulo expression.
local a = 5
return 12 % a
