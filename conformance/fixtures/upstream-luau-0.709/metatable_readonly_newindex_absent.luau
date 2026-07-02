-- Derived from Luau 0.709 tests/conformance/events.luau readonly __newindex checks.
local ni = {}
local t = table.create(2)

t[1] = 42
t.a = 1
t.b = 2
t.b = nil

setmetatable(t, {
    __newindex = function(_, k, v)
        assert(v == 42)
        table.insert(ni, k)
    end,
})
table.freeze(t)

assert(not pcall(function()
    t.a = 42
end))
assert(not pcall(function()
    t[1] = 42
end))
assert(not pcall(function()
    local key = "a"
    t[key] = 42
end))
assert(not pcall(function()
    local key = 1
    t[key] = 42
end))

assert(pcall(function()
    t.b = 42
end))
assert(pcall(function()
    t.c = 42
end))
assert(pcall(function()
    local key = "b"
    t[key] = 42
end))
assert(pcall(function()
    local key = "c"
    t[key] = 42
end))
assert(pcall(function()
    t[2] = 42
end))
assert(pcall(function()
    t[3] = 42
end))
assert(pcall(function()
    local key = 2
    t[key] = 42
end))
assert(pcall(function()
    local key = 3
    t[key] = 42
end))

local expected = {"b", "c", "b", "c", 2, 3, 2, 3}
assert(#ni == #expected)
for i = 1, #expected do
    assert(ni[i] == expected[i])
end

return "OK"
