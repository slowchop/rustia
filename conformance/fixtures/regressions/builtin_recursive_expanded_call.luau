local function run(n)
    if n == 0 then return 4, 5 end
    return string.len('abc', run(n - 1))
end
string.len = function(...) return select('#', ...), 7 end
local self = tostring(run(20))
local left, right
left = function(n)
    if n == 0 then return 4, 5 end
    return string.len('abc', right(n - 1))
end
right = function(n)
    if n == 0 then return 4, 5 end
    return string.len('abc', left(n - 1))
end
local callable = setmetatable({}, {
    __call = function(receiver, n)
        if n == 0 then return 4, 5 end
        return string.len('abc', receiver(n - 1))
    end,
})
local seen = 0
local function argument()
    seen = seen + 1
    return 'abc'
end
local function ordinary(n)
    local unused = string.len(argument())
    if n == 0 then return 0 end
    return 1 + ordinary(n - 1)
end
local after_expansion = ordinary(20)
assert(seen == 21)
return self .. ':' .. tostring(left(20)) .. ':' .. tostring(callable(20)) .. ':' .. after_expansion
