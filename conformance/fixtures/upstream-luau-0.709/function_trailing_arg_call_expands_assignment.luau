-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return
-- argument adjustment cases.

local function f()
    return 1, 2, 30, 4
end

local function ret2(a, b)
    return a, b
end

local a, b, c, d = ret2(f()), ret2(f())
assert(a == 1 and b == 1 and c == 2 and d == nil)
