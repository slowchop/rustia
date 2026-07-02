-- Derived from Luau 0.709 tests/conformance/calls.luau incorrect-argument table.sort row.
local a = {10, 9, 8, 4, 19, 23, 0, 0}
table.sort(a, function(a, b)
    return a < b
end, "extra arg")
return table.concat(a, ",")
