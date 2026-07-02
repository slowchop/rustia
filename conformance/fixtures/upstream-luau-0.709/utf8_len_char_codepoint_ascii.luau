local s = "hello World"
local t = {string.byte(s, 1, -1)}
assert(utf8.len(s) == #t)
assert(utf8.char(table.unpack(t)) == s)
local t1 = {utf8.codepoint(s, 1, -1)}
assert(#t1 == #t)
for i = 1, #t do
    assert(t[i] == t1[i])
end
return "OK"
