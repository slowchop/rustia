-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.pow(2, 0) == 1)
assert(math.pow(2, 2) == 4)
assert(math.pow(4, 0.5) == 2)
assert(math.pow(-2, 2) == 4)
assert(math.pow("2", 2) == 4)
assert(tostring(math.pow(-2, 0.5)) == "nan")
