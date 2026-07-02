-- Derived from Luau 0.709 tests/conformance/pcall.luau deferred protected-call upvalue closing.
local upclo

local function uptest(y)
    local a, b, c, d = 1, 2, 3, 4
    upclo = function()
        local t = table.pack("a", "b", "d", "e")
        assert(a == 1)
        assert(b == 2)
        assert(c == 3)
        assert(d == 4)
        a, b, c, d = table.unpack(t)
        return "ok"
    end
    y()
    error("oops")
end

local co = coroutine.create(function()
    pcall(uptest, coroutine.yield)
    return upclo()
end)

assert(coroutine.resume(co))
local done, result = coroutine.resume(co)
return done and result == "ok" and coroutine.status(co) == "dead"
