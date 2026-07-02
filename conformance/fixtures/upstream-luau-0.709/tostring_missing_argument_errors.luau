-- Derived from Luau 0.709 tests/conformance/strings.luau tostring arity checks.
local function nothing() end

assert(pcall(tostring) == false)
assert(pcall(function()
    return tostring(nothing())
end) == false)

return "OK"
