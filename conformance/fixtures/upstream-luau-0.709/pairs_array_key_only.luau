-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers pairs over an array table with one visible key variable.
local a = ''
for k in pairs({5, 6, 7}) do
    a = a .. k
end
return a
