-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers zero-step numeric for treating tonumber("0") as backward iteration.
local zero = tonumber("0")
local c = 0
for i = 1, 10, zero do
    c += 1
    if c > 10 then
        break
    end
end
return c
