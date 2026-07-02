assert(string.gsub("\252lo \252lo", "\252", "x") == "xlo xlo")
assert(string.gsub("alo \250lo  ", " +$", "") == "alo \250lo")
assert(string.gsub("  alo alo  ", "^%s*(.-)%s*$", "%1") == "alo alo")

local t = "ab\231 d"
local a, b = string.gsub(t, "(.)", "%1@")
assert("@" .. a == string.gsub(t, "", "@") and b == 5)

a, b = string.gsub("ab\231d", "(.)", "%0@", 2)
assert(a == "a@b@\231d" and b == 2)

assert(string.gsub("alo alo", "()[al]", "%1") == "12o 56o")
assert(string.gsub("abc=xyz", "(%w*)(%p)(%w+)", "%3%2%1-%0") == "xyz=abc-abc=xyz")
assert(string.gsub("abc", "%w", "%1%0") == "aabbcc")
assert(string.gsub("abc", "%w+", "%0%1") == "abcabc")
assert(string.gsub("\231\231\231", "$", "\000\231\231") == "\231\231\231\000\231\231")
assert(string.gsub("", "^", "r") == "r")
assert(string.gsub("", "$", "r") == "r")

assert(string.gsub("um (dois) tres (quatro)", "(%(%w+%))", string.upper) == "um (DOIS) tres (QUATRO)")

local words = { n = 0 }
assert(string.gsub("first second word", "%w+", function(w)
    words.n = words.n + 1
    words[words.n] = w
end, 2) == "first second word")
assert(words[1] == "first" and words[2] == "second" and words[3] == nil)

local function isbalanced(s)
    return string.find(string.gsub(s, "%b()", ""), "[()]") == nil
end

assert(isbalanced("(9 ((8))(\0) 7) \0\0 a b ()(c)() a"))
assert(not isbalanced("(9 ((8) 7) a b (\0 c) a"))
assert(string.gsub("alo 'oi' alo", "%b''", "\"") == "alo \" alo")

assert(not pcall(string.gsub, "alo", "(.", print))
assert(not pcall(string.gsub, "alo", ".)", print))
assert(not pcall(string.gsub, "alo", "(.", {}))
assert(not pcall(string.gsub, "alo", "(.)", "%2"))
assert(not pcall(string.gsub, "alo", "(%1)", "a"))
assert(not pcall(string.gsub, "alo", "(%0)", "a"))

return "OK"
