-- Derived from Luau 0.709 tests/conformance/calls.luau lines 52-57.

local a = { b = { c = {} } }

function a.b.c:f2(x, y)
    self[x] = y
end

a.b.c:f2("k", 12)
return a.b.c.k
