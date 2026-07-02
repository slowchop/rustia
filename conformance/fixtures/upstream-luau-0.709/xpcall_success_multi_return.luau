-- Derived from Luau 0.709 tests/conformance/pcall.luau xpcall success result preservation.
local t = table.pack(xpcall(function(a, b)
    return a, b, 42
end, error, 1, 2))

return t.n == 4 and t[1] == true and t[2] == 1 and t[3] == 2 and t[4] == 42
