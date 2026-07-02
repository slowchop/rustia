local function f()
    return 2, 3
end

local function g(a, b)
    return a, b
end

local function h(a, b, c)
    return a * 100 + b * 10 + c
end

return h(1, g(f()))
