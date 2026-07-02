-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers ipairs stopping at the first nil array element while reading the value.
local a = ''
for k, v in ipairs({5, 6, 7, nil, 8}) do
    a = a .. v
end
return a
