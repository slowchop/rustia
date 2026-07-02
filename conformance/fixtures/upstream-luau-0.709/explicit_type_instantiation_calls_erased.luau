-- Derived from Luau 0.709 tests/conformance/explicit_type_instantiations.luau.
local function identity<T>(x: T): T
    return x
end

local function multipleReturns<T>(x: T): (T, T)
    return x, x
end

local function typePacks<T...>(...: T...): T...
    return ...
end

local t = {}
function t:methodTypePacks<T...>(...: T...): T...
    assert(self == t)
    return ...
end

assert(identity<<number>>(1) == 1)
local a, b = multipleReturns<<number>>(2)
assert(a == 2 and b == 2)
local c, d = typePacks<<(string, number)>>("x", 3)
assert(c == "x" and d == 3)
local e, f = t:methodTypePacks<<(string, number)>>("y", 4)
assert(e == "y" and f == 4)
return "OK"
