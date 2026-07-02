-- Derived from Luau 0.709 tests/conformance/errors.luau empty-statement syntax checks.
assert(loadstring(";") == nil)
assert(loadstring("a=1;;") == nil)
assert(loadstring("return;;") == nil)
return "OK"
