-- Derived from Luau 0.709 tests/conformance/basic.luau line 54.

return (function()
    local a = {}
    local b = a
    a[1], a = 43, -1
    return a + b[1]
end)()
