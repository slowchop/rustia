-- Derived from Luau 0.709 tests/conformance/strings.luau and tpack.luau high-byte escape checks.
assert(string.byte("\255") == 255)
assert(string.byte("\xff") == 255)
assert(string.byte("\xe4") > 127)
assert(#"\0\xe4\0" == 3)
return "OK"
