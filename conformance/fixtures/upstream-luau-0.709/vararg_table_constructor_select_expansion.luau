-- Derived from Luau 0.709 tests/conformance/vararg.luau selectmany cases.
local function selectmany(n, ...)
    return table.concat({select(n, ...)}, ",")
end

return selectmany(2, 10, 20, 30)
