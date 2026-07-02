-- Derived from Luau 0.709 tests/conformance/constructs.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers an elseif chain falling through to the else block.
local x = 0
if false then
    x = 1
elseif false then
    x = 2
else
    x = 3
end
return x
