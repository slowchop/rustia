-- Derived from Luau 0.709 tests/conformance/tables.luau table.find __eq checks.
local objameta = {
    __eq = function(a, b)
        return a.x == b.x and a.y == b.y
    end,
}

local function makeobja(x, y)
    return setmetatable({x = x, y = y}, objameta)
end

local t3 = {}
table.insert(t3, makeobja(1, 2))
table.insert(t3, makeobja(2, 3))
table.insert(t3, makeobja(3, 4))

assert(table.find(t3, makeobja(1, 2)) == 1)
assert(table.find(t3, makeobja(2, 3)) == 2)
assert(table.find(t3, makeobja(3, 4)) == 3)

local function addnrec(n)
    return if n == 1 then 1 else n + addnrec(n - 1)
end

local objbmeta = {
    __eq = function(a, b)
        return addnrec(a.d) == addnrec(b.d)
    end,
}

local function makeobjb(d)
    return setmetatable({d = d}, objbmeta)
end

local o = makeobjb(8)
local t4 = {o, o, o, o, o, o, o, o}

assert(table.find(t4, makeobjb(8)) == 1)
assert(table.find(t4, makeobjb(12)) == nil)

return "OK"
