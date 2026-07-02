-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall immediate error handling.
local ok, msg = xpcall(function()
    error("foo")
end, function(err)
    return err:sub(err:find(": ") + 2, #err)
end)

return not ok and msg == "foo"
