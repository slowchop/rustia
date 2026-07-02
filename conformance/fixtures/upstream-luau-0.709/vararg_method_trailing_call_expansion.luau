local t = { base = 100 }

local function f()
    return 2, 3
end

function t:m(a, b)
    return self.base + a * 10 + b
end

return t:m(f())
