-- Derived from Luau 0.709 table.sort default less-than behavior for string arrays.
local a = {"b", "c", "a"}
table.sort(a)
return table.concat(a, ",")
