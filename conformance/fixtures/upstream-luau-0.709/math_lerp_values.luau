-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.lerp(1, 5, 0) == 1)
assert(math.lerp(1, 5, 1) == 5)
assert(math.lerp(1, 5, 0.5) == 3)
assert(math.lerp(1, 5, 1.5) == 7)
assert(math.lerp(1, 5, -0.5) == -1)
assert(math.lerp("1", "5", 0.5) == 3)

