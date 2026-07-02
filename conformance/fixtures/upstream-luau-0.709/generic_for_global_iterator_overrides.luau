-- Derived from Luau 0.709 tests/conformance/iter.luau iterator environment substitution checks.
local original_pairs, original_ipairs, original_next = pairs, ipairs, next

pairs = function()
    return "nope"
end

ipairs = function()
    return "nope"
end

next = {1, 2, 3}

local ok = pcall(function()
    for _ in pairs({}) do
    end
end)
assert(not ok)

ok = pcall(function()
    for _ in ipairs({}) do
    end
end)
assert(not ok)

ok = pcall(function()
    for _ in next, {} do
    end
end)
assert(not ok)

pairs, ipairs, next = original_pairs, original_ipairs, original_next

return "OK"
