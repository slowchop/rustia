-- Derived from Luau 0.709 tests/conformance/pcall.luau resumeerror helper coverage.
local co = coroutine.create(function()
    local ok, err = pcall(function()
        coroutine.yield()
    end)
    coroutine.yield("parked")
    return ok, err
end)

local ok, value = coroutine.resume(co)
assert(ok and value == nil and coroutine.status(co) == "suspended")

resumeerror(co, "fail")
assert(coroutine.status(co) == "suspended")

local done, protected, err = coroutine.resume(co)
return done and protected == false and err == "fail" and coroutine.status(co) == "dead"
