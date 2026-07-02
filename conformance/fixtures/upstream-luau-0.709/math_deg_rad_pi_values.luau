-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.pi - math.pi % 1 == 3)
assert(math.pi - math.pi % 0.001 == 3.141)
assert(math.deg(math.pi / 2) == 90)
assert(math.rad(90) == math.pi / 2)
assert(math.deg("0") == 0)
assert(math.rad("0") == 0)
