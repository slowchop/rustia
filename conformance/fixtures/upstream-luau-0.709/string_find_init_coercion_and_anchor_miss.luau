assert(string.find("abc", "b", "2") == 2)
assert(string.find("abc", "b", 2.9) == 2)

local justone = "^" .. utf8.charpattern .. "$"
assert(not string.find("A", justone, 2))
assert(not string.find("\a\b\f\n\r\t\v", "^%c%c%c%c%c%c%c$", 2))
assert(not string.match("false, x attempt", "^false,.*attempt", 2))

return "OK"
