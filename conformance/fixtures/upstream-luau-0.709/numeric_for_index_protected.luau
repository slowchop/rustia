-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the internal numeric for index being protected from loop-variable assignment.
local a = 1
for b = 9, 1, -2 do
    a = a * 2
    b = nil
end
return a
