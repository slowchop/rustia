-- Derived from Luau 0.709 tests/conformance/errors.luau recursive stack overflow checks.
local function y()
    y()
end

local ok, message = pcall(y)
assert(not ok)
assert(string.find(message, "^.-:%d+: stack overflow"))

return "OK"
