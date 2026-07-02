-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return
-- argument adjustment cases.

local function f()
    return 4, 5
end

local function ret2(a, b)
    return a, b
end

local a, b = ret2(f())
return a * 10 + b
