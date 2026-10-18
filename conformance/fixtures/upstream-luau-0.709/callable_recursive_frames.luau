-- Luau 0.709 callable receivers recurse through ordinary function metamethods.
local left, right = {}, {}
setmetatable(left, {__call = function(self, n)
    assert(self == left)
    if n == 0 then return 0 end
    return 1 + right(n - 1)
end})
setmetatable(right, {__call = function(self, n)
    assert(self == right)
    if n == 0 then return 0 end
    return 1 + left(n - 1)
end})
assert(left(40) == 40)
return true
