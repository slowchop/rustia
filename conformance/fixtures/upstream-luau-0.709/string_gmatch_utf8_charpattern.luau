-- Derived from Luau 0.709 tests/conformance/utf8.luau utf8.charpattern gmatch loop.
local lengths = {}

for c in string.gmatch("Aé\0漢", utf8.charpattern) do
    table.insert(lengths, tostring(#c))
end

return table.concat(lengths, ",")
