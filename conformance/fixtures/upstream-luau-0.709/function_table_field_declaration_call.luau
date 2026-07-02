-- Derived from Luau 0.709 tests/conformance/calls.luau lines 52-56.

local a = { b = { c = {} } }

function a.b.c.f1(x)
    return x + 1
end

return a.b.c.f1(4)
