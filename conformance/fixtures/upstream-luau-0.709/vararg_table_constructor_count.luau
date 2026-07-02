local function f(...)
    local t = { ... }
    return table.getn(t)
end

return f(1, 2, 3)
