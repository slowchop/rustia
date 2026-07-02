-- Derived from Luau 0.709 tests/conformance/pcall.luau recursive yield tests.
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield(1)
        return pcall(function()
            coroutine.yield(2)
            return 3
        end)
    end)
end)

local ok, value = coroutine.resume(co)
assert(ok and value == 1)

ok, value = coroutine.resume(co)
assert(ok and value == 2)

local done, outer, inner, result = coroutine.resume(co)
return done and outer and inner and result == 3 and coroutine.status(co) == "dead"
