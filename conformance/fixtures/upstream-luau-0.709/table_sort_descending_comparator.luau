-- Derived from Luau 0.709 table.sort custom comparator behavior.
local a = {1, 2, 3}
table.sort(a, function(a, b)
    return a > b
end)
return table.concat(a, ",")
