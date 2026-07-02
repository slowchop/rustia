-- Derived from Luau 0.709 tests/conformance/iter.luau generic for over non-iterable object checks.
local ok = pcall(function()
    for _ in 42 do
    end
end)

return ok
