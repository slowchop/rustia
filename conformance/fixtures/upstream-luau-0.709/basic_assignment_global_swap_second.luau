-- Derived from Luau 0.709 tests/conformance/basic.luau line 47.

return (function()
    local a = 1
    b = 2
    a, b = b, a
    return b
end)()
