-- Derived from Luau 0.709 tests/conformance/basic.luau and events.luau __eq checks.
local function concat(...)
    local out = {}
    for i = 1, select("#", ...) do
        out[i] = tostring(select(i, ...))
    end
    return table.concat(out, ",")
end

assert((function()
    local a = {}
    setmetatable(a, {})
    return concat(a == a, a ~= a)
end)() == "true,false")

assert((function()
    local a = {}
    local b = {}
    local mt = {__eq = function(l, r) return #l == #r end}
    setmetatable(a, mt)
    setmetatable(b, mt)
    return concat(a == b, a ~= b)
end)() == "true,false")

assert((function()
    local a = {}
    local b = {}
    local function eq(l, r)
        return #l == #r
    end
    setmetatable(a, {__eq = eq})
    setmetatable(b, {__eq = eq})
    return concat(a == b, a ~= b)
end)() == "true,false")

assert((function()
    local a = {}
    local b = {}
    setmetatable(a, {__eq = function(l, r) return #l == #r end})
    setmetatable(b, {__eq = function(l, r) return #l == #r end})
    return concat(a == b, a ~= b)
end)() == "false,true")

local t = {}
local function Set(x)
    local y = {}
    for _, k in pairs(x) do
        y[k] = 1
    end
    return setmetatable(y, t)
end

t.__eq = function(a, b)
    for k in pairs(a) do
        if not b[k] then
            return false
        end
        b[k] = nil
    end
    return next(b) == nil
end

local s = Set{1, 3, 5}
assert(s == Set{3, 5, 1})
assert(not rawequal(s, Set{3, 5, 1}))
assert(rawequal(s, s))
assert(Set{1, 3, 5, 1} == Set{3, 5, 1})
assert(Set{1, 3, 5} ~= Set{3, 5, 1, 6})
t[Set{1, 3, 5}] = 1
assert(t[Set{1, 3, 5}] == nil)

do
    local tbl = {}
    local ud = newproxy(true)

    setmetatable(tbl, {__eq = print})
    getmetatable(ud).__eq = print

    assert(tbl ~= tbl)
    assert(ud ~= ud)
end

return "OK"
