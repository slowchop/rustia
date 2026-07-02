-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall + pcall + yield case.
local co = coroutine.create(function()
    return xpcall(pcall, function(...)
        return ...
    end, function()
        return pcall(function()
            coroutine.yield(42)
        end)
    end)
end)

local ok, value = coroutine.resume(co)
assert(ok and value == 42)

local done, xpcall_ok, pcall_ok, inner_ok = coroutine.resume(co)
return done and xpcall_ok and pcall_ok and inner_ok and coroutine.status(co) == "dead"
