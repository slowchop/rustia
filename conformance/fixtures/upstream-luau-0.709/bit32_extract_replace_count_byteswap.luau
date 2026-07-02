-- Derived from Luau 0.709 tests/conformance/bitwise.luau extract/replace/count/byteswap checks.
assert(bit32.extract(0x12345678, 0, 4) == 8)
assert(bit32.extract(0x12345678, 4, 4) == 7)
assert(bit32.extract(0xa0001111, 31, 1) == 1)
assert(bit32.extract(0xf2345679, 0, 32) == 0xf2345679)
assert(bit32.extract(42, 1, 3) == 5)

assert(bit32.replace(0x12345678, 5, 28, 4) == 0x52345678)
assert(bit32.replace(0x12345678, 0x87654321, 0, 32) == 0x87654321)
assert(bit32.replace(-1, 0, 1, 2) == 2^32 - 7)

assert(bit32.countlz(0) == 32)
assert(bit32.countlz(42) == 26)
assert(bit32.countrz(0) == 32)
assert(bit32.countrz(42) == 1)
assert(bit32.byteswap(0x10203040) == 0x40302010)
assert(bit32.byteswap(-1) == 0xffffffff)

return "OK"
