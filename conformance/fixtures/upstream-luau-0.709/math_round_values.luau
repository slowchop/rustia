-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
assert(math.round(0) == 0)
assert(math.round(0.4) == 0)
assert(math.round(0.5) == 1)
assert(math.round(3.5) == 4)
assert(math.round(-0.4) == 0)
assert(math.round(-0.5) == -1)
assert(math.round(-3.5) == -4)
assert(math.round(math.huge) == math.huge)
assert(math.round(0.49999999999999994) == 0)
assert(math.round(-0.49999999999999994) == 0)
assert(math.round("1.8") == 2)

