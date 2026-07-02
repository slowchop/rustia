-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream repeat/until loop doubling accumulator example.
local a = 10
local b = 1
repeat
    b = b * 2
    a = a - 1
until a == 1
return b
