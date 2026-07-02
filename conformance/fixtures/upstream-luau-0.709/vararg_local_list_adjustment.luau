local function f(a, ...)
    local b, c, d = ...
    return a * 100 + b * 10 + c + (d or 0)
end

return f(1, 2, 3)
