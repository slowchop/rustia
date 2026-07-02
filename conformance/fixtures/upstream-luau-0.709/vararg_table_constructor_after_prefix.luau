local function f(...)
    local t = { 9, ... }
    return table.getn(t) * 100 + t[1] * 10 + t[4]
end

return f(1, 2, 3)
