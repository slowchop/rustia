-- Derived from Luau 0.709 tests/conformance/calls.luau nested table function and method declarations.
local a = {b = {c = {}}}

function a.b.c.f1(x)
    return x + 1
end

function a.b.c:f2(x, y)
    self[x] = y
end

a.b.c:f2("k", 12)
return a.b.c.f1(4) + a.b.c.k
