assert(string.char() == "")
assert(string.byte(string.char(255)) == 255)
assert(string.byte(string.char(0)) == 0)
assert(string.byte(string.char(65.9)) == 65)
assert(string.byte(string.char(-0.9)) == 0)
assert(string.char(0, 255, 0) == "\0\255\0")
assert(string.char(0, string.byte("\xe4"), 0) == "\0\xe4\0")
assert(string.char(string.byte("\xe4l\0óu", 1, -1)) == "\xe4l\0óu")
assert(string.char(string.byte("\xe4l\0óu", 1, 0)) == "")
assert(pcall(function() return string.char(256) end) == false)
assert(pcall(function() return string.char(-1) end) == false)

return "OK"
