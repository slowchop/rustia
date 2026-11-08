-- Focused Luau 0.709 oracle regression for issue #18.
do
    local mt = { __add = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t + 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __add = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t + 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "12", e)
end
do
    local mt = { __add = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t + 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "24", e)
end
do
    local mt = { __sub = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t - 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __sub = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t - 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "39", e)
end
do
    local mt = { __sub = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t - 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "51", e)
end
do
    local mt = { __mul = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t * 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __mul = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t * 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "66", e)
end
do
    local mt = { __mul = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t * 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "78", e)
end
do
    local mt = { __div = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t / 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __div = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t / 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "93", e)
end
do
    local mt = { __div = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t / 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "105", e)
end
do
    local mt = { __mod = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t % 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __mod = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t % 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "120", e)
end
do
    local mt = { __mod = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t % 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "132", e)
end
do
    local mt = { __pow = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t ^ 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __pow = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t ^ 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "147", e)
end
do
    local mt = { __pow = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t ^ 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "159", e)
end
do
    local mt = { __unm = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return -t
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __unm = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return -t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "174", e)
end
do
    local mt = { __unm = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return -t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "186", e)
end
do
    local mt = { __len = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return #t
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __len = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return #t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "201", e)
end
do
    local mt = { __len = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return #t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "213", e)
end
do
    local mt = { __concat = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t .. t
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __concat = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t .. t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "228", e)
end
do
    local mt = { __concat = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t .. t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "240", e)
end
do
    local mt = { __eq = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t == setmetatable({}, mt)
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __eq = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t == setmetatable({}, mt)
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "255", e)
end
do
    local mt = { __eq = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t == setmetatable({}, mt)
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "267", e)
end
do
    local mt = { __lt = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t < t
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __lt = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t < t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "282", e)
end
do
    local mt = { __lt = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t < t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "294", e)
end
do
    local mt = { __le = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t <= t
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __le = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t <= t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "309", e)
end
do
    local mt = { __le = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t <= t
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "321", e)
end
do
    local mt = { __index = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t.missing
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __index = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t.missing
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "336", e)
end
do
    local mt = { __index = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        return t.missing
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "348", e)
end
do
    local mt = { __newindex = function() error("bad", 0) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        t.missing = 1
    end)
    assert(not ok)
    assert(e == "bad")
end
do
    local mt = { __newindex = function() error("bad", 1) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        t.missing = 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "363", e)
end
do
    local mt = { __newindex = function() error("bad", 2) end }
    local t = setmetatable({}, mt)
    local ok, e = pcall(function()
        t.missing = 1
    end)
    assert(not ok)
    assert(string.match(e, ":(%d+):") == "375", e)
end
local t = setmetatable({}, {__add = function()
    local _, line = debug.info(2, "sl")
    return line
end})
assert(t + 1 == 384)
return "OK"
