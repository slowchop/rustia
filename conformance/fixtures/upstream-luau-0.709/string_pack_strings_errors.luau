-- Derived from Luau 0.709 tests/conformance/tpack.luau string and error checks.
local pack = string.pack
local unpack = string.unpack
local packsize = string.packsize

assert(pack("zB", "abc", 247) == "abc\0\xF7")
local s, b, pos = unpack("zB", "abc\0\xF9")
assert(s == "abc" and b == 249 and pos == 6)

assert(pack("c5", "ab") == "ab\0\0\0")
assert(unpack("c3", "abcdef") == "abc")
assert(pack("s2", "hello") == "\5\0hello")
assert(unpack("s2", "\5\0hello") == "hello")

local ok, err = pcall(pack, "z", "a\0")
assert(not ok and string.find(err, "contains zeros"))
ok, err = pcall(packsize, "z")
assert(not ok and string.find(err, "variable-length format", 1, true))
ok, err = pcall(pack, "!4i3", 0)
assert(not ok and string.find(err, "not power of 2"))
ok, err = pcall(pack, "Xi17")
assert(not ok and string.find(err, "%(17%) out of limits %[1,16%]"))
ok, err = pcall(packsize, string.rep("c268435456", 8))
assert(not ok and string.find(err, "too large"))
assert(packsize("c1073741824") == 2^30)
ok, err = pcall(unpack, "i987654321", "")
assert(not ok and string.find(err, "out of limits"))
ok, err = pcall(unpack, "i9876543210", "")
assert(not ok and string.find(err, "too large"))

return "OK"
