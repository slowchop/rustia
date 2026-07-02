-- Derived from Luau 0.709 tests/conformance/tables.luau foreach/foreachi checks.
local t = {x = 90, y = 8, z = 23}
assert(table.foreach(t, function(k, v)
    if k == "x" then
        return v
    end
end) == 90)
assert(table.foreach(t, function(k, v)
    if k == "a" then
        return v
    end
end) == nil)

local called = false
table.foreach({}, function()
    called = true
end)
assert(not called)

local counter = {n = 1}
table.foreachi({n = 3}, function(i, v)
    assert(counter.n == i and v == nil)
    counter.n = counter.n + 1
end)
assert(counter.n == 4)

local a = {10, 20, 30}
local sum = 0
table.foreachi(a, function(i, v)
    assert(a[i] == v)
    sum = sum + v
end)
assert(sum == 60)

assert(table.foreachi({"a", "b", "c"}, function(i, v)
    if i == 2 then
        return v
    end
end) == "b")

return "OK"
