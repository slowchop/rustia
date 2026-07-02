-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall missing-handler error.
local ok, err = pcall(xpcall, function()
    return 42
end)

return not ok and err == "missing argument #2 to 'xpcall' (function expected)"
