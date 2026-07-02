assert(bit32.lshift(1, 1.5) == 2)
assert(bit32.rrotate(0x12345678, "4.9") == 0x81234567)
assert(bit32.extract(0xff, 0.9, 4.1) == 0xf)
assert(bit32.replace(0, 0xff, 4.9, 4.1, "ignored") == 0xf0)
assert(bit32.band(2251799813685249, 0xffffffff) == 1)

return "OK"
