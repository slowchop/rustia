-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers numeric for counting down with a negative step.
local a = 1
for b = 9, 1, -2 do
    a = a * 2
end
return a
