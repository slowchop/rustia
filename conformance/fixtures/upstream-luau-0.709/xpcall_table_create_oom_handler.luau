-- Derived from Luau 0.709 tests/conformance/pcall.luau OOM xpcall handler cases.
local ok, err = xpcall(function()
    table.create(1e6)
end, function(e)
    return e
end)

return ok == false and err == "not enough memory"
