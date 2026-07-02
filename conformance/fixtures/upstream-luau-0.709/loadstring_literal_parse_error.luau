-- Derived from Luau 0.709 tests/conformance/errors.luau loadstring parse-error check.
local chunk = loadstring("a=9+")
assert(chunk == nil)

return "OK"
