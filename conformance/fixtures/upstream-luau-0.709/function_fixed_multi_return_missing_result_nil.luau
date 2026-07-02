-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return cases.

local function f()
    return 1, 2
end

local a, b, c = f()
return c
