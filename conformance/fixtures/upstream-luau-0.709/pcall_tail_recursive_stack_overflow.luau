-- Derived from Luau 0.709 tests/conformance/pcall.luau stackinfinite check.
local function stackinfinite()
    return stackinfinite()
end

local ok, message = pcall(stackinfinite)
return ok == false and type(message) == "string" and string.find(message, "stack overflow", 1, true) ~= nil
