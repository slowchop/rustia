-- Derived from Luau 0.709 tests/conformance/strings.luau %* tostring metamethod checks.
local hits = 0
local object = setmetatable({}, {
    __tostring = function(self)
        assert(type(self) == "table")
        hits = hits + 1
        return "ok"
    end,
})

assert(getmetatable(object).__tostring)
assert(tostring(object) == "ok")
assert(string.format("%*", object) == "ok")
assert(hits == 2)

local userdata = newproxy(true)
assert(type(userdata) == "userdata")

local mt = getmetatable(userdata)
mt.__tostring = function(self)
    assert(type(self) == "userdata")
    return "good"
end

assert(tostring(userdata) == "good")
assert(string.format("%*", userdata) == "good")

local clone = newproxy(userdata)
assert(getmetatable(clone) == mt)
assert(string.format("%*", clone) == "good")

return "OK"
