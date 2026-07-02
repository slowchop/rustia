-- Derived from Luau 0.709 tests/conformance/pcall.luau OOM in function and handler cases.
local ok, err = xpcall(function()
    table.create(1e6)
end, function()
    table.create(1e6)
end)

return ok == false and err == "not enough memory"
