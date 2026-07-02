-- Derived from Luau 0.709 tests/conformance/bitwise.luau shift and rotate checks.
assert(bit32.lrotate(0x12345678, 4) == 0x23456781)
assert(bit32.rrotate(0x12345678, -4) == 0x23456781)
assert(bit32.lrotate(0x12345678, -8) == 0x78123456)
assert(bit32.rrotate(0x12345678, 8) == 0x78123456)

assert(bit32.lshift(0x12345678, 4) == 0x23456780)
assert(bit32.lshift(0x12345678, -4) == 0x01234567)
assert(bit32.lshift(0x12345678, 32) == 0)
assert(bit32.rshift(0x12345678, 4) == 0x01234567)
assert(bit32.rshift(0x12345678, -32) == 0)

assert(bit32.arshift(0x12345678, 1) == 0x12345678 / 2)
assert(bit32.arshift(-1, 32) == 0xffffffff)
assert(bit32.arshift(-1, -1) == (-1 * 2) % 2^32)

return "OK"
