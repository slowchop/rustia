-- Derived from Luau 0.709 tests/conformance/iter.luau pairs substitution error-message check.
pairs = function()
    return "nope"
end

local ok, err = pcall(function()
    for _ in pairs({}) do
    end
end)

assert(not ok and err:match("attempt to iterate over a string value"))
return "OK"
