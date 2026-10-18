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
return self .. ':' .. tostring(left(20))
