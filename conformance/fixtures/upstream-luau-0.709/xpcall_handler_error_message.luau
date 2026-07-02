-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall error-in-handler collapse.
local ok, msg = xpcall(function()
    error("foo")
end, function(_err)
    error("bar")
end)

if ok then
    return "unexpected success"
end

return msg
