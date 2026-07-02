-- Derived from Luau 0.709 tests/conformance/calls.luau x = {unpack(a)} checks.
local x = {table.unpack({1, 2, 3}, 2, 3)}

if x[1] == 2 and x[2] == 3 then
    return table.getn(x)
end
return -1
