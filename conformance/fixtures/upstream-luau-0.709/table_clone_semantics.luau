-- Derived from Luau 0.709 tests/conformance/tables.luau table.clone checks.
local t = {a = 1, b = 2, 3, 4, 5}
local tt = table.clone(t)

assert(#tt == 3)
assert(tt.a == 1 and tt.b == 2)

t.c = 3
assert(tt.c == nil)

t = table.freeze({"test"})
tt = table.clone(t)
assert(table.isfrozen(t) and not table.isfrozen(tt))
assert(tt[1] == "test")
assert(not pcall(setmetatable, t, {}))

local mt = {}
t = setmetatable({}, mt)
tt = table.clone(t)
assert(getmetatable(t) == getmetatable(tt))

t = setmetatable({}, {__metatable = "protected"})
assert(getmetatable(t) == "protected")
assert(not pcall(table.freeze, t))
assert(not pcall(table.clone, t))
assert(not pcall(table.clone))
assert(not pcall(table.clone, 42))

return "OK"
