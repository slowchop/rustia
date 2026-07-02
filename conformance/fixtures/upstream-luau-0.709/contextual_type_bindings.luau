-- Derived from Luau 0.709 tests/conformance/gc.luau contextual `type` local binding.
local type = _G.type

local function calltype(type)
    return type(1)
end

for type in next, {true} do
    return calltype(_G.type) == "number" and type == 1 and "OK" or "FAIL"
end

return "FAIL"
