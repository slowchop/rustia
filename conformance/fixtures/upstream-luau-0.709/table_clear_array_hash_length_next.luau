-- Derived from Luau 0.709 tests/conformance/clear.luau table.clear array/hash removal.
local t = {1, 2, 3}
t.a = 4
table.clear(t)
if #t ~= 0 then
    return -1
end
if next(t) ~= nil then
    return -2
end
return 1
