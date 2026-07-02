-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall invalid-handler error.
local ok, err = pcall(xpcall, function()
    return 42
end, true)

return not ok and err == "invalid argument #2 to 'xpcall' (function expected, got boolean)"
