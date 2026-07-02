local justone = "^" .. utf8.charpattern .. "$"
assert(string.find("A", justone) == 1)
assert(not string.find("A", justone, 2))
local start, finish = string.find("\xE6\xBC\xA2", justone)
assert(start == 1 and finish == 3)
assert(not string.find("\xE6\xBC\xA2A", justone))
assert(not string.find("\x80", justone))
return "OK"
