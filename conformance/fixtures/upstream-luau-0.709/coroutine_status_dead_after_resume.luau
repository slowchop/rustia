-- Derived from Luau 0.709 tests/conformance/coroutine.luau coroutine.status rows.
local thread = coroutine.create(function()
end)
assert(coroutine.status(thread) == "suspended")
assert(coroutine.resume(thread))
return coroutine.status(thread)
