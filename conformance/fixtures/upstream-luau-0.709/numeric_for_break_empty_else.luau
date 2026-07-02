-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers break from a numeric for loop with an empty else block.
local a = 1
for b = 1, 9 do
    a = a * 2
    if a == 128 then
        break
    else
    end
end
return a
