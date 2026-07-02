-- Derived from Luau 0.709 tests/conformance/basic.luau line 53.

return (function()
    local a, b = 1, {}
    a, b[a] = 43, -1
    return a + b[1]
end)()
