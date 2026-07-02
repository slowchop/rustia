-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.min(1) == 1)
assert(math.min(1, 2) == 1)
assert(math.min(1, 2, -1) == -1)
assert(math.min(1, -1, 2) == -1)
assert(math.min(1, -1, 2, -2) == -2)
assert(math.max(1) == 1)
assert(math.max(1, 2) == 2)
assert(math.max(1, 2, -1) == 2)
assert(math.max(1, -1, 2) == 2)
assert(math.max(1, -1, 2, -2) == 2)
assert(math.max("1", 2) == 2)
assert(math.max(2, "1") == 2)
assert(math.max(1, 2, "3") == 3)
assert(math.min("1", 2) == 1)
assert(math.min(2, "1") == 1)
assert(math.min(1, 2, "3") == 1)

