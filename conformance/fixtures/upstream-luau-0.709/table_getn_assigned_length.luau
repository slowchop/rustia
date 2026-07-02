-- Derived from Luau 0.709 tests/conformance/tables.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local a = {}
for j = 1, 40 do
    a[j] = j
end
return table.getn(a)
