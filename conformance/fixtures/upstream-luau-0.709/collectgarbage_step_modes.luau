-- Derived from Luau 0.709 tests/conformance/gc.luau collectgarbage step mode checks.
local function dosteps(size)
    collectgarbage()
    collectgarbage("stop")
    local x = gcinfo()
    local i = 0
    repeat
        i += 1
    until collectgarbage("step", size)
    assert(gcinfo() < x)
    return i
end

assert(type(collectgarbage("stop")) == "number")
assert(type(collectgarbage("restart")) == "number")
assert(dosteps(0) > 10)
assert(dosteps(6) < dosteps(2))
assert(dosteps(10000) == 1)

return "OK"
