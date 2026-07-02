-- Derived from Luau 0.709 tests/conformance/iter.luau getfenv iterator substitution checks.
local env = getfenv(1)
local original_pairs, original_ipairs, original_next = env.pairs, env.ipairs, env.next

env.pairs = function()
    return "nope"
end

env.ipairs = function()
    return "nope"
end

env.next = {1, 2, 3}

local ok, err = pcall(function()
    for _ in pairs({}) do
    end
end)
assert(not ok and err:match("attempt to iterate over a string value"))

ok, err = pcall(function()
    for _ in ipairs({}) do
    end
end)
assert(not ok and err:match("attempt to iterate over a string value"))

ok, err = pcall(function()
    for _ in next, {} do
    end
end)
assert(not ok and err:match("attempt to iterate over a table value"))

env.pairs, env.ipairs, env.next = original_pairs, original_ipairs, original_next

return "OK"
