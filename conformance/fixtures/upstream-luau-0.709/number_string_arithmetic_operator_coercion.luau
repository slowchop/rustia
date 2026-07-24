-- Derived from Luau 0.709 VM/src/lvmutils.cpp arithmetic coercion.
assert(-"5" == -5)
assert(-"  0x10  " == -16)
assert(-"0x1p4" == -16)
assert(tostring(-"-0") == "0")

assert("8" - "3" == 5)
assert("6" / "2" == 3)
assert("7" // "2" == 3)
assert("-7" // "2" == -4)
assert("7" % "4" == 3)
assert("2" ^ "3" == 8)

local ok = pcall(function()
    return -"0b101"
end)
assert(not ok)

return "OK"
