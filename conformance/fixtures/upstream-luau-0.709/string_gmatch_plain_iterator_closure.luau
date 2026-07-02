-- Derived from Luau 0.709 tests/conformance/pm.luau string.gmatch iterator use.
local f = string.gmatch("bananana", "na")
assert(f() == "na")
assert(f() == "na")
assert(f() == "na")
assert(f() == nil)
return "OK"
