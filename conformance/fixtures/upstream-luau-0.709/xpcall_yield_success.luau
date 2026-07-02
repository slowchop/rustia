-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall yielding success case.
local co = coroutine.create(function()
    return xpcall(function()
        coroutine.yield("pause")
        return 42
    end, error)
end)

local ok, value = coroutine.resume(co)
assert(ok and value == "pause")

local done, protected, result = coroutine.resume(co)
return done and protected and result == 42 and coroutine.status(co) == "dead"
