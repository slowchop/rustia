-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers tonumber("nan") numeric for step iterating once when backward bounds match.
local nan = tonumber("nan")
local c = 0
for i = 10, 1, nan do
    c += 1
end
return c
