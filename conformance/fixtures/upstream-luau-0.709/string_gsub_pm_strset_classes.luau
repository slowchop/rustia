local t = {}
for i = 0, 255 do
    t[#t + 1] = i
end
local abc = string.char(table.unpack(t))

local function strset(p)
    local res = { s = "" }
    string.gsub(abc, p, function(c)
        res.s = res.s .. c
    end)
    return res.s
end

assert(string.len(strset("[\200-\210]")) == 11)
assert(strset("[a-z]") == "abcdefghijklmnopqrstuvwxyz")
assert(strset("[a-z%d]") == strset("[%da-uu-z]"))
assert(strset("[a-]") == "-a")
assert(strset("[^%W]") == strset("[%w]"))
assert(strset("[]%%]") == "%]")
assert(strset("[a%-z]") == "-az")
assert(strset("[%^%[%-a%]%-b]") == "-[]^ab")
assert(strset("%Z") == strset("[\1-\255]"))
assert(strset(".") == strset("[\1-\255%z]"))

return "OK"
