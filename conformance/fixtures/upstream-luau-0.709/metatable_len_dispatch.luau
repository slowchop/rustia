-- Derived from Luau 0.709 tests/conformance/events.luau __len/rawlen checks.
local t = {1}

setmetatable(t, {})
assert(#t == 1)

setmetatable(t, {__len = rawlen})
assert(#t == 1)

setmetatable(t, {
    __len = function()
        return 42
    end,
})
assert(#t == 42)

setmetatable(t, {__len = 42})
local ok, err = pcall(function()
    return #t
end)
assert(not ok and err:match("attempt to call a number value"))

setmetatable(t, {
    __len = function() end,
})
ok, err = pcall(function()
    return #t
end)
assert(not ok and err:match("'__len' must return a number"))

setmetatable(t, {__len = error})
ok, err = pcall(function()
    return #t
end)
assert(not ok and err == t)

setmetatable(t, {__len = 42})
assert(rawlen(t) == 1)
assert(rawlen("foo") == 3)

ok, err = pcall(function()
    return rawlen(42)
end)
assert(not ok and err:match("table or string expected"))

return "OK"
