-- Derived from Luau 0.709 tests/conformance/errors.luau newline-sensitive call syntax.
assert(loadstring("a=math.sin\n(3)") == nil)
return "OK"
