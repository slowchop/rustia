-- Derived from Luau 0.709 tests/conformance/iter.luau direct table iteration checks.
local dictionary_sum = 0
for _, v in {a = 1, b = 2, c = 3} do
    dictionary_sum += v
end
assert(dictionary_sum == 6)

local array_sum = 0
for _, v in {1, 2, 3, nil, 5} do
    array_sum += v
end
assert(array_sum == 11)

local mixed_sum = 0
for _, v in {1, 2, 3, nil, 5, a = 1, b = 2, c = 3} do
    mixed_sum += v
end
assert(mixed_sum == 17)

local key_sum = 0
for k in {1, 2, 3, 4, 5} do
    key_sum += k
end
assert(key_sum == 15)

local extra_sum = 0
for k, _, a, b, c, d, e in {1, 2, 3, 4, 5} do
    extra_sum += k
    assert(a == nil and b == nil and c == nil and d == nil and e == nil)
end
assert(extra_sum == 15)

return "OK"
