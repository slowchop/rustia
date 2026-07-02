-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers numeric for exiting when infinity plus negative infinity produces NaN.
local c = 0
for i = math.huge, math.huge, -math.huge do
    c += 1
end
return c
