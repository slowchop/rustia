-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local a = {}
for k, v in next, { a = 1, b = 2, c = 3 } do
    a[k] = 1
end
return a.a + a.b + a.c
