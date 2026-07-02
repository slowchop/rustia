-- Derived from Luau 0.709 tests/conformance/bitwise.luau coercion and error checks.
assert(bit32.lrotate("0x12345678", 4) == 0x23456781)
assert(bit32.arshift("-1", 32) == 0xffffffff)
assert(bit32.bnot("1") == 0xfffffffe)
assert(bit32.band("1", 3) == 1)
assert(bit32.bor(1, "2") == 3)
assert(bit32.replace(-1, 0, 1, "2") == 2^32 - 7)

assert(not pcall(bit32.band, {}))
assert(not pcall(bit32.bnot, "a"))
assert(not pcall(bit32.lshift, 45))
assert(not pcall(bit32.extract, 0, -1))
assert(not pcall(bit32.extract, 0, 31, 2))

return "OK"
