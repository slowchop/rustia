-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers continue in a repeat/until loop.
local a = 1
repeat
    a = a + 1
    if a < 5 then
        continue
    end
    a = a * 2
until a > 100
return a
