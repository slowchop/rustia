-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.map(0, -1, 1, 0, 2) == 1)
assert(math.map(1, 1, 4, 0, 2) == 0)
assert(math.map(2.5, 1, 4, 0, 2) == 1)
assert(math.map(4, 1, 4, 0, 2) == 2)
assert(math.map(1, 1, 4, 2, 0) == 2)
assert(math.map(2.5, 1, 4, 2, 0) == 1)
assert(math.map(4, 1, 4, 2, 0) == 0)
assert(math.map(1, 4, 1, 2, 0) == 0)
assert(math.map(2.5, 4, 1, 2, 0) == 1)
assert(math.map(4, 4, 1, 2, 0) == 2)
assert(math.map(-8, 0, 4, 0, 2) == -4)
assert(math.map(16, 0, 4, 0, 2) == 8)

