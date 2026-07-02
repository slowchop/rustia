-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return cases.

local function f()
    return 1, 2, 30, 4
end

local a, b, c, d = f()
return a * 1000000 + b * 10000 + c * 100 + d
