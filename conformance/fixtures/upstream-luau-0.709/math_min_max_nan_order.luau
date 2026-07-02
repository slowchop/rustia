-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local nan = 0 / 0
assert(math.min(nan, 2) ~= math.min(nan, 2))
assert(math.min(1, nan) == 1)
assert(math.max(nan, 2) ~= math.max(nan, 2))
assert(math.max(1, nan) == 1)

