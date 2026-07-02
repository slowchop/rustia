-- Derived from Luau 0.709 tests/conformance/pcall.luau deferred OOM protected-call cases.
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield()
        table.create(1e6)
    end)
end)

assert(coroutine.resume(co))

local done, ok, err = coroutine.resume(co)
return done and ok == false and err == "not enough memory"
