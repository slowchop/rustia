assert(utf8.len("abc", 1, -1, "ignored") == 3)
assert(utf8.codepoint("abc", 1, 1, "ignored") == 97)
assert(utf8.offset("abc", 2, 1, "ignored") == 2)

local f = utf8.codes("A")
local p, c = f("A")
assert(p == 1 and c == 65)

local t = {}
for i = 1, 300 do
    t[i] = 97
end
local wide = utf8.char(table.unpack(t))
assert(#wide == 300)

return "OK"
