-- Derived from Luau 0.709 tests/conformance/native_types.luau function type-pack annotations.
local function checkfunction(a: () -> ())
    return a
end

type Callback = () -> ()

local callback: Callback = function()
end

return checkfunction(callback) == callback and "OK" or "FAIL"
