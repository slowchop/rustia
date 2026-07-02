-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
local inf = math.huge
assert(math.exp("0") == 1)
assert(math.log10("10") == 1)
assert(math.log("0") == -inf)
assert(math.log("8", 2) == 3)
assert(math.log("10", 10) == 1)
assert(math.log("16", 4) == 2)
