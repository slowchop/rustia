-- Derived from Luau 0.709 tests/conformance/pcall.luau pcall success result-list checks.
local t = table.pack(pcall(function(a, b)
    return a, b, 42
end, 1, 2))

if t[1] == true and t[2] == 1 and t[3] == 2 and t[4] == 42 then
    return t.n
end
return -1
