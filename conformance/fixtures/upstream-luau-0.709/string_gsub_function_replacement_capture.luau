local p = string.gsub("a%1 b%8", "%%([0-9])", function(s)
    return "%" .. (s + 1)
end)
assert(p == "a%2 b%9")

local seen = ""
local result, count = string.gsub("a1b2", "[0-9]", function(c)
    seen = seen .. c
end)
assert(result == "a1b2" and seen == "12" and count == 2)

return p
