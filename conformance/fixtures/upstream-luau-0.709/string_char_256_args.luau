local function range(i, j)
    if i <= j then
        return i, range(i + 1, j)
    end
end

local s = string.char(range(0, 255))
assert(#s == 256)
assert(string.byte(s, 1) == 0)
assert(string.byte(s, 256) == 255)

return #s
