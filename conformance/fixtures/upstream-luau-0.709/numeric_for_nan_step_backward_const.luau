-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers NaN numeric for step iterating once when backward bounds match.
local c = 0
for i = 10, 1, 0 / 0 do
    c += 1
end
return c
