-- Derived from Luau 0.709 tests/conformance/events.luau protected setmetatable check.
local a = {}
setmetatable(a, {
    __metatable = "xuxu",
    __tostring = function(x)
        return x.name
    end,
})

assert(getmetatable(a) == "xuxu")
assert(not pcall(setmetatable, a, {}))
a.name = "gororoba"
assert(tostring(a) == "gororoba")

return "OK"
