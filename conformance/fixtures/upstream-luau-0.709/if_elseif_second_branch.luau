-- Derived from Luau 0.709 tests/conformance/constructs.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers an elseif chain selecting the first truthy elseif branch.
local x = 0
if false then
    x = 1
elseif true then
    x = 2
else
    x = 3
end
return x
