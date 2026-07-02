local a, b, c = string.byte("abc", 1, 3)
local d, e, f = string.byte("abc", -10, 100)
assert(d == a and e == b and f == c)
return a * 10000 + b * 100 + c
