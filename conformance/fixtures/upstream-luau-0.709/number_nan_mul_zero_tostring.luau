-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Uses a local to keep the arithmetic path runtime-visible.
local a = 0 / 0
return tostring(a * 0)
