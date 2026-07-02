-- Derived from Luau 0.709 tests/conformance/iter.luau generic for with __iter checks.
local f = {}
setmetatable(f, {
    __iter = function(value)
        assert(value == f)
        return next, {1, 2, 3, 4}
    end,
})

local table_sum = 0
for n in f do
    table_sum += n
end
assert(table_sum == 10)

local u = newproxy(true)
getmetatable(u).__iter = function(value)
    assert(value == u)
    return next, {1, 2, 3, 4}
end

local userdata_sum = 0
for n in u do
    userdata_sum += n
end
assert(userdata_sum == 10)

return "OK"
