-- Derived from Luau 0.709 tests/conformance/events.luau proxy checks.
assert(getmetatable(newproxy()) == nil)
assert(getmetatable(newproxy(false)) == nil)
assert(getmetatable(newproxy(nil)) == nil)

local u = newproxy(true)
assert(type(u) == "userdata")

local mt = getmetatable(u)
assert(type(mt) == "table")

mt.__newindex = function(self, key, value)
    assert(self == u)
    getmetatable(self)[key] = value
end

mt.__index = function(self, key)
    assert(self == u)
    return getmetatable(self)[key]
end

for i = 1, 10 do
    u[i] = i
end

for i = 1, 10 do
    assert(u[i] == i)
end

mt.__metatable = "xuxu"
assert(getmetatable(u) == "xuxu")
assert(select(2, pcall(getmetatable, u)) == "xuxu")

return "OK"
