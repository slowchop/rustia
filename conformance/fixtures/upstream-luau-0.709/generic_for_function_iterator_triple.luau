-- Derived from Luau 0.709 tests/conformance/iter.luau generic for with function iterators.
local unpack = table.unpack

local function f(n, p)
    local t = {}
    for i = 1, p do
        t[i] = i * 10
    end
    return function(_, control)
        if control > 0 then
            control = control - 1
            return control, unpack(t)
        end
    end, nil, n
end

local x = 0
for n, a, b, c, d in f(5, 3) do
    x += 1
    assert(a == 10 and b == 20 and c == 30 and d == nil)
    assert(n == 5 - x)
end

return x
