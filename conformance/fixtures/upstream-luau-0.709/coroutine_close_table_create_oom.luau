-- Derived from Luau 0.709 tests/conformance/pcall.luau coroutine.close OOM case.
local co = coroutine.create(function()
    table.create(1e6)
end)

coroutine.resume(co)

local ok, err = coroutine.close(co)
return ok == false and err == "not enough memory"
