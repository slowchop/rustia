-- Derived from Luau 0.709 tests/conformance/sort.luau __lt table.sort check.
local mt = {
    __lt = function(a, b)
        return a.val < b.val
    end,
}

local a = {
    setmetatable({val = 5}, mt),
    setmetatable({val = 1}, mt),
    setmetatable({val = 3}, mt),
    setmetatable({val = 2}, mt),
    setmetatable({val = 4}, mt),
}

table.sort(a)

for i = 1, 5 do
    assert(a[i].val == i)
end

for i = #a, 2, -1 do
    assert(not mt.__lt(a[i], a[i - 1]))
    assert(not (a[i] < a[i - 1]))
end

return "OK"
