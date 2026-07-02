-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers generic for keeping its internal pairs index protected from visible-variable assignment.
local a = 1
for b in pairs({1, 2, 3}) do
    a = a * 2
    b = nil
end
return a
