-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return
-- adjustment cases.

local function tail()
    return 2, 3
end

local function f()
    return 1, tail()
end

local a, b, c = f()
return a * 100 + b * 10 + c
