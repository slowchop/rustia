-- Derived from Luau 0.709 tests/conformance/pcall.luau error-after-yield cases.
local co = coroutine.create(function()
    return pcall(function()
        coroutine.yield()
        error("foo")
    end)
end)

assert(coroutine.resume(co))

local done, protected, err = coroutine.resume(co)
return done and not protected and type(err) == "string" and string.find(err, "foo", 1, true) ~= nil
