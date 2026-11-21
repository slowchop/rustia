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
-- Instantiation is also a value expression. Like parentheses it selects one result.
local f = identity<<number>>
assert(f(10) == 10)
local first, second = multipleReturns(11)<<number>>
assert(first == 11 and second == nil)
local function record() return {value = 12} end
assert(record()<<number>>.value == 12)
-- An instantiated receiver is evaluated before an argument can rebind its local.
local function value(self) return self.id end
local original = {value = value, id = 1}
local replacement = {value = value, id = 2}
local receiver = original
local function swap() receiver = replacement; return 0 end
assert(receiver<<number>>:value((swap())) == 1)
assert(receiver == replacement)
receiver = original
assert((receiver):value((swap())) == 2)

-- A captured builtin alias is an ordinary callee when instantiated.
local op = math.abs
local function changeOp() op = math.floor; return -1.5 end
assert(op<<number>>((changeOp())) == 1.5)
assert(op(-1.5) == -2)
return "OK"
