-- Derived from Luau 0.709 table.sort behavior exercised by tests/conformance/calls.luau.
local a = {3, 1, 2}
table.sort(a)
return table.concat(a, ",")
