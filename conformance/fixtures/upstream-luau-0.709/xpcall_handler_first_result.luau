-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall handler result adjustment.
local t = table.pack(xpcall(function()
    error("foo")
end, function(_err)
    return "bar", "extra"
end))

return t.n == 2 and t[1] == false and t[2] == "bar"
