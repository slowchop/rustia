-- Derived from Luau 0.709 tests/conformance/calls.luau bare unpack checks.
local values = {1, 2, nil, 4}
local packed = table.pack(unpack(values, 1, 4))

if packed[1] == 1 and packed[2] == 2 and packed[3] == nil and packed[4] == 4 then
    return packed.n
end
return -1
