-- Derived from Luau 0.709 tests/conformance/pcall.luau OOM xpcall handler override case.
local ok, err = xpcall(function()
    table.create(1e6)
end, function()
    return "oops"
end)

return ok == false and err == "oops"
