-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers parallel local assignment preserving old RHS values: a, b = b, a.
local a = 1
local b = 2
a, b = b, a
return b
