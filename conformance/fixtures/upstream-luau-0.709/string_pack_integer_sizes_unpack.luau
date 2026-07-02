-- Derived from Luau 0.709 tests/conformance/tpack.luau integer pack/unpack size checks.
local pack = string.pack
local unpack = string.unpack
local packsize = string.packsize

assert(packsize("b") == 1)
assert(packsize("h") == 2)
assert(packsize("i") == 4)
assert(packsize("l") == 8)
assert(packsize("j") == 4)
assert(packsize("T") == 4)

assert(unpack("B", pack("B", 0xff)) == 0xff)
assert(unpack("b", pack("b", 0x7f)) == 0x7f)
assert(unpack("b", pack("b", -0x80)) == -0x80)
assert(unpack("H", pack("H", 0xffff)) == 0xffff)
assert(unpack("h", pack("h", -0x8000)) == -0x8000)
assert(unpack("L", pack("L", 0xffffffff)) == 0xffffffff)
assert(unpack("l", pack("l", -0x80000000)) == -0x80000000)

return "OK"
