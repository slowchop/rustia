-- Derived from Luau 0.709 tests/conformance/calls.luau multiple-return
-- adjustment cases.

local function ret2(a, b)
    return a, b
end

local a, b, c, d = ret2(1, 2), ret2(3, 4)
if d == nil then
    return a * 100 + b * 10 + c
else
    return 0
end
