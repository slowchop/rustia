-- Derived from Luau 0.709 tests/conformance/errors.luau stack traceback scanning.
local lengths = {}

for line in string.gmatch("a\n\n", "[^\n]*") do
    table.insert(lengths, tostring(#line))
end

local empty_count = 0
for line in string.gmatch("", "[^\n]*") do
    empty_count = empty_count + 1
    assert(line == "")
end

return table.concat(lengths, ",") .. "|" .. tostring(empty_count)
