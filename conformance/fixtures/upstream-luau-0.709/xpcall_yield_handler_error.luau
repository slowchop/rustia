-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall handler-error-after-yield case.
local co = coroutine.create(function()
    return xpcall(function()
        coroutine.yield()
        error("foo")
    end, function()
        error("bar")
    end)
end)

assert(coroutine.resume(co))

local done, protected, err = coroutine.resume(co)
return done and not protected and err == "error in error handling"
