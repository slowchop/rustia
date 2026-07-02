-- Derived from Luau 0.709 tests/conformance/calls.luau recursive unlpack
-- multiple-return case.

local function unlpack(t, i)
    i = i or 1
    if i <= table.getn(t) then
        return t[i], unlpack(t, i + 1)
    end
end

local a, b, c, d = unlpack({ 1, 2, 3 })
assert(a == 1 and b == 2 and c == 3 and d == nil)
