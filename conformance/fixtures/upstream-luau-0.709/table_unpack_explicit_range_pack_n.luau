-- Derived from Luau 0.709 tests/conformance/calls.luau unpack(a, lim-2) range checks.
local t = table.pack(table.unpack({1, 2, 3}, 2, 3))

if t[1] == 2 and t[2] == 3 then
    return t.n
end
return -1
