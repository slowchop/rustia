-- Derived from Luau 0.709 tests/conformance/coroutine.luau coroutine.create/resume rows.
local thread = coroutine.create(function(value)
    return value + 1
end)
local ok, value = coroutine.resume(thread, 41)
assert(ok)
return value
