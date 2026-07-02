assert(string.match("alo xyzK", "(%w+)K") == "xyz")
assert(string.match("254 K", "(%d*)K") == "")
assert(string.match("alo ", "(%w*)$") == "")
assert(string.match("alo ", "(%w+)$") == nil)

local a, b, c, d, e = string.match("\255lo alo", "^(((.).).* (%w*))$")
assert(a == "\255lo alo" and b == "\255l" and c == "\255" and d == "alo" and e == nil)

a, b, c, d = string.match("0123456789", "(.+(.?)())")
assert(a == "0123456789" and b == "" and c == 11 and d == nil)

return "OK"
