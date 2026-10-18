-- These fixed-arity Luau 0.709 functions evaluate and ignore trailing arguments.
local values = type(1, 99) .. ';' .. typeof(1, 99)
    .. ';' .. tonumber('1', 10, 99)
    .. ';' .. next({a = 1}, nil, 99)
    .. ';' .. table.concat({1}, '', 1, 1, 99)
    .. ';' .. table.getn({4}, 99)
    .. ';' .. table.maxn({4}, 99)
    .. ';' .. table.create(1, 4, 99)[1]
    .. ';' .. table.find({4}, 4, 1, 99)
    .. ';' .. table.remove({4}, 1, 99)
    .. ';' .. string.sub('abc', 1, 2, 99)
    .. ';' .. string.len('abc', 99)
    .. ';' .. string.byte('abc', 1, 1, 99)
    .. ';' .. string.upper('abc', 99)
    .. ';' .. string.lower('ABC', 99)
    .. ';' .. string.reverse('abc', 99)
    .. ';' .. string.find('abc', 'a', 1, true, 99)
    .. ';' .. string.split('a,b', ',', 99)[1]
local order = 0
local function argument(n)
    order = order * 10 + n
    return n
end
return values .. ';' .. type(argument(1), argument(2), argument(3)) .. ':' .. order
