-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers continue in a numeric for loop.
local a = 1
for i = 1, 8 do
    a = a + 1
    if a < 5 then
        continue
    end
    a = a * 2
end
return a
