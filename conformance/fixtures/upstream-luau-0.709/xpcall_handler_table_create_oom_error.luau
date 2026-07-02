-- Derived from Luau 0.709 tests/conformance/pcall.luau OOM in xpcall handler cases.
local ok, err = xpcall(function()
    error("oops")
end, function()
    table.create(1e6)
end)

return ok == false and err == "error in error handling"
