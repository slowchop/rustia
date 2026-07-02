-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers break from a repeat loop before an always-false condition.
local a = 10
local b = 1
repeat
    b = b * 2
    a = a - 1
    if a == 1 then
        break
    end
until false
return b
