-- Derived from Luau 0.709 tests/conformance/basic.luau line 46.

return (function()
    local a = 1
    b = 2
    a, b = b, a
    return a
end)()
