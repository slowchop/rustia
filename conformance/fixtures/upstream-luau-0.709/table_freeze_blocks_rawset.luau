-- Derived from Luau 0.709 tests/conformance/tables.luau readonly rawset checks.
local t = table.freeze({1})
return rawset(t, 1, 2)
