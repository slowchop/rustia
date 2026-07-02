-- Derived from Luau 0.709 tests/conformance/errors.luau return-position syntax checks.
assert(loadstring("return 1\nlocal x = 2") == nil)
return "OK"
