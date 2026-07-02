-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream if statement where a truthy local executes the then block.
local a = 1
if a then
    a = 2
end
return a
