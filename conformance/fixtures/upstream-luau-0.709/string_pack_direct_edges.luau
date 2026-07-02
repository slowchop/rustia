local s = string.pack("B", 0xff)
local value, position = string.unpack("B", s, 0)

if value == 0xff and position == 2 and string.packsize("c1073741824") == 1073741824 then
    return "OK"
end

return "BAD"
