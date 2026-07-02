-- Derived from Luau 0.709 tests/conformance/strings.luau quote, char, and float formatting.
local x = '"ílo"\n\\'
assert(string.format("%q%s", x, x) == '"\\"ílo\\"\\\n\\\\""ílo"\n\\')
assert(string.format("%q", "\0") == [["\000"]])
assert(string.format("%q", "\r") == [["\r"]])
assert(string.format("\0%c\0%c%x\0", string.byte("\xe4"), string.byte("b"), 140) == "\0\xe4\0b8c\0")
assert(string.format("%c%c%c%c", 1, 0, 2, 3) == "\1\0\2\3")
assert(string.format("%5c%-5c", 1, 2) == "    \1\2    ")
assert(string.format("%e %E", 1.5, -1.5) == "1.500000e+00 -1.500000E+00")
assert(tonumber(string.format("%f", 10.3)) == 10.3)
assert(pcall(string.format, "%##################d", 1) == false)
assert(pcall(string.format, "%.123d", 1) == false)
assert(pcall(string.format, "%?", 1) == false)

return "OK"
