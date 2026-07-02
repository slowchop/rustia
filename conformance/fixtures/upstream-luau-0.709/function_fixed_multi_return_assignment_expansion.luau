-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return cases.

local function f()
    return 1, 2
end

local a, b = 0, 0
a, b = f()
return a * 10 + b
