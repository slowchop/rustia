-- Derived from Luau 0.709 tests/conformance/pcall.luau deferred OOM xpcall handler cases.
local co = coroutine.create(function()
    return xpcall(function()
        coroutine.yield()
        table.create(1e6)
    end, function(e)
        return e
    end)
end)

assert(coroutine.resume(co))

local done, ok, err = coroutine.resume(co)
return done and ok == false and err == "not enough memory"
