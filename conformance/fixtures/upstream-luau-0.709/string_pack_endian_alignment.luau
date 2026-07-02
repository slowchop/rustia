-- Derived from Luau 0.709 tests/conformance/tpack.luau endian and alignment checks.
local pack = string.pack
local unpack = string.unpack
local packsize = string.packsize

assert(pack("<i2 >i2", 10, 20) == "\10\0\0\20")
local a, b, pos = unpack("<i2 >i2", "\10\0\0\20")
assert(a == 10 and b == 20 and pos == 5)

assert(pack(" < i1 i2 ", 2, 3) == "\2\3\0")
assert(pack("!4 i1 i2", 2, 3) == "\2\0\3\0")
assert(packsize("!4 xXi4") == 4)

return "OK"
