-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers manual generic for over next with key/value variables, reading the key.
local a = ''
for k, v in next, {5, 6, 7} do
    a = a .. k
end
return a
