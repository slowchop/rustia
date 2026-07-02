-- Derived from Luau 0.709 tests/conformance/move.luau move-to-new-table checks.
local a = {}
local result = table.move({10, 20, 30}, 1, 3, 1, a)

if result == a then
    return table.concat(a, ",")
end
return "bad"
