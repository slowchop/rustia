-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers array table holes and absent keys through the upstream concat helper.
local a = {4, 5, nil, 6}
return concat(a[1], a[3], a[4], a[100])
