-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local inf = math.huge * 2
local nan = 0 / 0
assert(math.sign(0) == 0)
assert(math.sign(42) == 1)
assert(math.sign(-42) == -1)
assert(math.sign(inf) == 1)
assert(math.sign(-inf) == -1)
assert(math.sign(nan) == 0)
assert(math.sign("2") == 1)
assert(math.sign("-2") == -1)
assert(math.sign("0") == 0)

