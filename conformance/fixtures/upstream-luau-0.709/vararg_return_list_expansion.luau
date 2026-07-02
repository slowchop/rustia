local function f(...)
    return 1, ...
end

local a, b, c = f(2, 3)
return a * 100 + b * 10 + c
