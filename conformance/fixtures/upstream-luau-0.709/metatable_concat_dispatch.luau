-- Derived from Luau 0.709 tests/conformance/events.luau __concat checks.
local mt = {}
local returns_string

mt.__concat = function(a, b, c)
    assert(c == nil)
    if type(a) == "table" then
        a = a.val
    end
    if type(b) == "table" then
        b = b.val
    end
    if returns_string then
        return a .. b
    else
        return setmetatable({val = a .. b}, mt)
    end
end

local c = setmetatable({val = "c"}, mt)
local d = setmetatable({val = "d"}, mt)

returns_string = true
assert(c .. d == "cd")
assert(0 .. "a" .. "b" .. c .. d .. "e" .. "f" .. (5 + 3) .. "g" == "0abcdef8g")

returns_string = false
local x = c .. d
assert(getmetatable(x) == mt and x.val == "cd")

x = 0 .. "a" .. "b" .. c .. d .. "e" .. "f" .. "g"
assert(x.val == "0abcdefg")

return "OK"
