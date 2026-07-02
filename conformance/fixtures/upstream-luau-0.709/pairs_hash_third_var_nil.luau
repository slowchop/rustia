-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local a = ''
for k, v, p in pairs({ a = 1, b = 2, c = 3 }) do
    a = a .. tostring(p)
end
return a
