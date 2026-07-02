-- Derived from Luau 0.709 tests/conformance/events.luau __lt/__le checks.
local t = {}

t.__lt = function(a, b, c)
    collectgarbage()
    assert(c == nil)
    if type(a) == "table" then a = a.x end
    if type(b) == "table" then b = b.x end
    return a < b, "dummy"
end

local function Op(x)
    return setmetatable({x = x}, t)
end

local function test()
    assert(not (Op(1) < Op(1)) and (Op(1) < Op(2)) and not (Op(2) < Op(1)))
    assert(not (Op("a") < Op("a")) and (Op("a") < Op("b")) and not (Op("b") < Op("a")))
    assert((Op(1) <= Op(1)) and (Op(1) <= Op(2)) and not (Op(2) <= Op(1)))
    assert((Op("a") <= Op("a")) and (Op("a") <= Op("b")) and not (Op("b") <= Op("a")))
    assert(not (Op(1) > Op(1)) and not (Op(1) > Op(2)) and (Op(2) > Op(1)))
    assert(not (Op("a") > Op("a")) and not (Op("a") > Op("b")) and (Op("b") > Op("a")))
    assert((Op(1) >= Op(1)) and not (Op(1) >= Op(2)) and (Op(2) >= Op(1)))
    assert((Op("a") >= Op("a")) and not (Op("a") >= Op("b")) and (Op("b") >= Op("a")))
end

test()

t.__le = function(a, b, c)
    assert(c == nil)
    if type(a) == "table" then a = a.x end
    if type(b) == "table" then b = b.x end
    return a <= b, "dummy"
end

test()

local t1, t2, c, d
t1 = {}
c = {}
setmetatable(c, t1)
d = {}
t1.__lt = function()
    return true
end

assert(not pcall(function()
    return c < d
end))

setmetatable(d, t1)
assert(c < d and not (d <= c))

t2 = {}
t2.__lt = t1.__lt
setmetatable(d, t2)
assert(c < d and not (d <= c))

return "OK"
