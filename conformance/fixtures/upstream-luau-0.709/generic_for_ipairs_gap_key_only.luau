-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers ipairs stopping at the first nil array element with one visible key variable.
local a = ''
for k in ipairs({5, 6, 7, nil, 8}) do
    a = a .. k
end
return a
