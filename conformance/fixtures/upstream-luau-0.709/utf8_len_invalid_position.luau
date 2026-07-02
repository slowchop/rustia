local a, b = utf8.len("abc\xE3def")
assert(not a and b == 4)
a, b = utf8.len("\x80hello")
assert(not a and b == 1)
return "OK"
