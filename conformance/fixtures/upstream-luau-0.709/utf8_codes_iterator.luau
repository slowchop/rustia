local s = "A\xC3\xA9\0\xE6\xBC\xA2"
local expected = {65, 233, 0, 28450}
local i = 0
for p, c in utf8.codes(s) do
    i += 1
    assert(c == expected[i])
    assert(p == utf8.offset(s, i))
end
assert(i == #expected)
local f = utf8.codes("")
assert(f("", 2) == nil)
assert(f("", -1) == nil)
return "OK"
