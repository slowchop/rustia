-- Derived from Luau 0.709 tests/conformance/vararg.luau select/unpack table-constructor expansion.
local unpack = table.unpack
local a = {select(3, unpack{10, 20, 30, 40})}

return table.concat(a, ",")
