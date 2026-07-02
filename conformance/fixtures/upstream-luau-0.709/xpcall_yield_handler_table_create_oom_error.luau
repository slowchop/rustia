-- Derived from Luau 0.709 tests/conformance/pcall.luau deferred OOM in xpcall handler case.
local co = coroutine.create(function()
    return xpcall(function()
        coroutine.yield()
        error("oops")
    end, function()
        table.create(1e6)
    end)
end)

assert(coroutine.resume(co))

local done, ok, err = coroutine.resume(co)
return done and ok == false and err == "error in error handling"
