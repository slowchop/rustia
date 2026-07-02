-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.fmod(3, 2) == 1)
assert(math.fmod(-3, 2) == -1)
assert(math.fmod(3, -2) == 1)
assert(math.fmod(-3, -2) == -1)
assert(math.fmod("1.5", 1) == 0.5)
