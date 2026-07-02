-- Derived from Luau 0.709 tests/conformance/tables.luau table freezing invariants.
local t = {}
if table.isfrozen(t) then
    return -1
end
t[1] = 1
t.a = 2
if table.freeze(t) ~= t then
    return -2
end
if not table.isfrozen(t) then
    return -3
end
return t[1] + t.a
