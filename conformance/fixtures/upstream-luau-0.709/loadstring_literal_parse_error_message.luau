-- Derived from Luau 0.709 tests/conformance/errors.luau doit(loadstring) helper.
local chunk, message = loadstring("a=9+")
assert(chunk == nil)
return type(message)
