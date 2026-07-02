-- Derived from Luau 0.709 tests/conformance/events.luau NaN/nil __newindex checks.
assert(not pcall(function()
    local t = {}
    t[nil] = 5
end))

assert(not pcall(function()
    local t = {}
    setmetatable(t, {__newindex = {}})
    t[nil] = 5
end))

assert(pcall(function()
    local t = {}
    setmetatable(t, {__newindex = function() end})
    t[nil] = 5
end))

assert(not pcall(function()
    local t = {}
    t[0 / 0] = 5
end))

assert(not pcall(function()
    local t = {}
    setmetatable(t, {__newindex = {}})
    t[0 / 0] = 5
end))

assert(pcall(function()
    local t = {}
    setmetatable(t, {__newindex = function() end})
    t[0 / 0] = 5
end))

return "OK"
