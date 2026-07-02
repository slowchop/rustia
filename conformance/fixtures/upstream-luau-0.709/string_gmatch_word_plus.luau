-- Derived from Luau 0.709 tests/conformance/pm.luau %w+ gmatch loop.
local words = {}

for word in string.gmatch("first second word", "%w+") do
    table.insert(words, word)
end

return table.concat(words, "|")
