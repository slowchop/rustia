-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers Luau truthiness where 0 is truthy and selects the then branch.
local a = 0
if a then
    a = 1
else
    a = 2
end
return a
