-- Derived from Luau 0.709 tests/conformance/assert.luau multi-argument assert return.
local t = table.pack(assert(1, nil, 3))
if t[1] == 1 and t[2] == nil and t[3] == 3 then
    return t.n
end
return -1
