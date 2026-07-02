-- Derived from Luau 0.709 tests/conformance/pcall.luau yieldable pcall success cases.
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield("pause")
        return 1, 2, 3
    end)
end)

local ok, value = coroutine.resume(co)
assert(ok and value == "pause")

local done, protected, a, b, c = coroutine.resume(co)
return done and protected and a == 1 and b == 2 and c == 3 and coroutine.status(co) == "dead"
