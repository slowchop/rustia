-- Derived from Luau 0.709 tests/conformance/calls.luau unpack result-list adjustment checks.
local t = table.pack(table.unpack({1, nil, 3}, 1, 3))

if t[1] == 1 and t[2] == nil and t[3] == 3 then
    return t.n
end
return -1
