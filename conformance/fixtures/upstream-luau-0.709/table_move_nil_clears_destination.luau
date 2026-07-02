-- Derived from Luau 0.709 tests/conformance/move.luau nil-containing range checks.
local a = {1, 2, 3, 4, 5}
table.move({10}, 1, 3, 2, a)

if a[1] == 1 and a[2] == 10 and a[3] == nil and a[4] == nil and a[5] == 5 then
    return 1
end
return -1
