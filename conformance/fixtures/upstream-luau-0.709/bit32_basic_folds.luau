-- Derived from Luau 0.709 tests/conformance/bitwise.luau fold operation checks.
assert(bit32.band() == bit32.bnot(0))
assert(bit32.btest() == true)
assert(bit32.bor() == 0)
assert(bit32.bxor() == 0)

assert(bit32.band(-1) == 0xffffffff)
assert(bit32.band(2^33 + 1) == 1)
assert(bit32.band(-2^40 - 2) == 0xfffffffe)
assert(bit32.band(1, 2) == 0)
assert(bit32.bor(1, 2, 4) == 7)
assert(bit32.bxor(1, 3, 5) == 7)
assert(bit32.btest(1, 3) == true)
assert(bit32.bnot(1) == 0xfffffffe)

return "OK"
