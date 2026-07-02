-- Derived from Luau 0.709 tests/conformance/events.luau __index/__newindex checks.
local mt = {}
local a = {10, 20, 30, x = "10"}

mt.__index = function(t, i)
    local parent = rawget(t, "parent")
    return parent[i] + 3, "ignored"
end

a.parent = {z = 25, x = 12, [4] = 24}
setmetatable(a, mt)
assert(a[1] == 10)
assert(a.z == 28)
assert(a[4] == 27)
assert(a.x == "10")

local b = setmetatable({}, mt)
mt.__newindex = function(t, i, v)
    rawset(t, i, v - 3)
end

b[1] = 30
b.x = "101"
b[5] = 200
assert(b[1] == 27 and b.x == 98 and b[5] == 197)

local c = {}
local d = setmetatable({}, mt)
mt.__newindex = c
d[1] = 10
d[2] = 20
d[3] = 90
assert(c[1] == 10 and c[2] == 20 and c[3] == 90)

local recursive
recursive = setmetatable({}, {
    __index = setmetatable({}, {
        __index = setmetatable({}, {
            __index = function(_, n)
                return recursive[n - 3] + 4, "ignored"
            end,
        }),
    }),
})
recursive[0] = 20
for i = 0, 10 do
    assert(recursive[i * 3] == 20 + i * 4)
end

return "OK"
