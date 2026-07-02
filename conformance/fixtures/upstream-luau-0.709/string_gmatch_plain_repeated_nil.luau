-- Derived from Luau 0.709 tests/conformance/pm.luau string.gmatch iterator use.
local f = string.gmatch("abc", "z")
assert(f() == nil)
assert(f() == nil)
return "OK"
