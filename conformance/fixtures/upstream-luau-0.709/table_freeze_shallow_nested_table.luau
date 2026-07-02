-- Derived from Luau 0.709 tests/conformance/tables.luau shallow table.freeze behavior.
local t = {child = {}}
local child = t.child
table.freeze(t)
child.x = 5
if table.isfrozen(child) then
    return -1
end
return child.x
