-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers assigning array table holes and distant keys, then reading through the upstream concat helper.
local a = {4, 5, nil, 6}
a[1] = 42
a[3] = 0
a[100] = 75
return concat(a[1], a[3], a[75], a[100])
