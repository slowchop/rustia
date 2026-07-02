-- Derived from Luau 0.709 tests/conformance/strings.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local total = 0
for i = 0, 30 do
    total += string.len(string.rep('a', i))
end
return total
