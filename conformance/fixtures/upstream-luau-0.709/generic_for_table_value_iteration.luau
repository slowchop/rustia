-- Derived from Luau 0.709 tests/conformance/sort.luau table value iteration checks.
local t = {1, 2, 3, nil, 5, a = 1, b = 2, c = 3}
local sum = 0

for _, value in t do
    sum += value
end

return sum
