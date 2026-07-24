-- Derived from Luau 0.709 VM/src/lbaselib.cpp luaB_newproxy argument checks.
local none = newproxy()
local nil_proxy = newproxy(nil)
local false_proxy = newproxy(false)
local true_proxy = newproxy(true)

assert(type(none) == "userdata")
assert(type(nil_proxy) == "userdata")
assert(type(false_proxy) == "userdata")
assert(type(true_proxy) == "userdata")

assert(getmetatable(none) == nil)
assert(getmetatable(nil_proxy) == nil)
assert(getmetatable(false_proxy) == nil)
assert(type(getmetatable(true_proxy)) == "table")
assert(getmetatable(true_proxy) ~= getmetatable(newproxy(true)))

local function rejects(value)
    local ok = pcall(newproxy, value)
    assert(not ok)
end

rejects(true_proxy)
rejects({})
rejects(1)
rejects("proxy")
rejects(function()
end)

return "OK"
