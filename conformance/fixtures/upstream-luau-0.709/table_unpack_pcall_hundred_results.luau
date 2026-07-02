-- Derived from Luau 0.709 tests/conformance/pcall.luau "returning lots of results" check.
local res = {pcall(function()
    return table.unpack(table.create(100, "a"))
end)}

if res[1] == true and res[2] == "a" and res[101] == "a" then
    return table.getn(res)
end
return -1
