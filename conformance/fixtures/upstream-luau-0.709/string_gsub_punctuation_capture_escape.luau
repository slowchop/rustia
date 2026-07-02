-- Derived from Luau 0.709 tests/conformance/errors.luau checksyntax token escaping.
local escaped, count = string.gsub("a+b.c?", "(%p)", "%%%1")
assert(count == 3)
return escaped
