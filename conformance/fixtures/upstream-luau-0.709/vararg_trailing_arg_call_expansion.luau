local function f(...)
    return ...
end

local function g(a, b, c)
    return a * 100 + b * 10 + c
end

return g(1, f(2, 3))
