-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream local/table assignment conflict row without wrapping it in a function.
local a, b = 1, {}
a, b[a] = 43, -1
return a + b[1]
