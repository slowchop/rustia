local function outer(...)
    local function g(a, b)
        return a, b
    end

    local function h(a, b, c)
        return a * 100 + b * 10 + c
    end

    return h(1, g(...))
end

return outer(2, 3)
