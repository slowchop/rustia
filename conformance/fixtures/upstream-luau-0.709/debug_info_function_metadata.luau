-- Derived from Luau 0.709 tests/conformance/debug.luau debug.info function metadata cases.
local function quuz(f)
    local a, v = debug.info(f, "a")
    return tostring(a) .. " " .. tostring(v)
end

assert(debug.info(math.sqrt, "n") == "sqrt")
assert(debug.info(math.sqrt, "s") == "[C]")
assert(debug.info(math.sqrt, "f") == math.sqrt)
assert(quuz(math.cos) == "0 true")
assert(quuz(function()
end) == "0 false")
assert(quuz(function(...)
end) == "0 true")
assert(quuz(function(a, b)
end) == "2 false")
assert(quuz(function(a, b, ...)
end) == "2 true")

local t = {
    foo = function()
        return 1
    end,
}
assert(debug.info(t.foo, "n") == "foo")
assert(debug.info(t.foo, "l") > 0)

return "OK"
