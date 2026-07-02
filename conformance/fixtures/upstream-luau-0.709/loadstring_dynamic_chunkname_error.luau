-- Derived from Luau 0.709 tests/conformance/errors.luau loadstring chunk-name check.
local chunk, message = loadstring("nope", "@short")
assert(chunk == nil)
return string.sub(message, 1, 8)
