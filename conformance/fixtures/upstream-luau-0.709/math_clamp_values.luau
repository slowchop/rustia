-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.clamp(-1, 0, 1) == 0)
assert(math.clamp(0.5, 0, 1) == 0.5)
assert(math.clamp(2, 0, 1) == 1)
assert(math.clamp(4, 0, 0) == 0)
assert(math.clamp("0", 2, 3) == 2)
assert(math.clamp("4", 2, 3) == 3)

