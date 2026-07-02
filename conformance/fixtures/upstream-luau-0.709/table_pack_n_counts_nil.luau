-- Derived from Luau 0.709 tests/conformance/pcall.luau table.pack(...).n checks.
local t = table.pack(1, nil, 3, nil)
if t[1] == 1 and t[2] == nil and t[3] == 3 and t[4] == nil then
    return t.n
end
return -1
