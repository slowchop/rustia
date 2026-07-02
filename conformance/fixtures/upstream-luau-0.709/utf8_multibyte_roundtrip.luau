local s = "\xE6\xB1\x89\xE5\xAD\x97/\xE6\xBC\xA2\xE5\xAD\x97"
local t = {27721, 23383, 47, 28450, 23383}
assert(utf8.len(s, 1, -1) == #t)
assert(utf8.char(table.unpack(t)) == s)
local t1 = {utf8.codepoint(s, 1, -1)}
assert(#t1 == #t)
for i = 1, #t do
    assert(t[i] == t1[i])
end
return "OK"
