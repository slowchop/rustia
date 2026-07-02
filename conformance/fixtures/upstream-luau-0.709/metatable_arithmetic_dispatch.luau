-- Derived from Luau 0.709 tests/conformance/events.luau arithmetic metamethod checks.
local mt = {}
local cap

local function f(op)
    return function(...)
        cap = {[0] = op, ...}
        return (...)
    end
end

mt.__add = f("add")
mt.__sub = f("sub")
mt.__mul = f("mul")
mt.__div = f("div")
mt.__idiv = f("idiv")
mt.__mod = f("mod")
mt.__unm = f("unm")
mt.__pow = f("pow")

local a = setmetatable({}, mt)
local b = setmetatable({}, mt)

assert(b + 5 == b)
assert(cap[0] == "add" and cap[1] == b and cap[2] == 5 and cap[3] == nil)
assert(b + "5" == b)
assert(cap[0] == "add" and cap[1] == b and cap[2] == "5" and cap[3] == nil)
assert(5 + b == 5)
assert(cap[0] == "add" and cap[1] == 5 and cap[2] == b and cap[3] == nil)
assert("5" + b == "5")
assert(cap[0] == "add" and cap[1] == "5" and cap[2] == b and cap[3] == nil)

b = b - 3
assert(getmetatable(b) == mt)
assert(5 - a == 5)
assert(cap[0] == "sub" and cap[1] == 5 and cap[2] == a and cap[3] == nil)
assert("5" - a == "5")
assert(cap[0] == "sub" and cap[1] == "5" and cap[2] == a and cap[3] == nil)

assert(a * a == a)
assert(cap[0] == "mul" and cap[1] == a and cap[2] == a and cap[3] == nil)
assert(a / 0 == a)
assert(cap[0] == "div" and cap[1] == a and cap[2] == 0 and cap[3] == nil)
assert(a // 0 == a)
assert(cap[0] == "idiv" and cap[1] == a and cap[2] == 0 and cap[3] == nil)
assert(a % 2 == a)
assert(cap[0] == "mod" and cap[1] == a and cap[2] == 2 and cap[3] == nil)
assert(-a == a)
assert(cap[0] == "unm" and cap[1] == a)
assert(a ^ 4 == a)
assert(cap[0] == "pow" and cap[1] == a and cap[2] == 4 and cap[3] == nil)
assert(a ^ "4" == a)
assert(cap[0] == "pow" and cap[1] == a and cap[2] == "4" and cap[3] == nil)
assert(4 ^ a == 4)
assert(cap[0] == "pow" and cap[1] == 4 and cap[2] == a and cap[3] == nil)
assert("4" ^ a == "4")
assert(cap[0] == "pow" and cap[1] == "4" and cap[2] == a and cap[3] == nil)

return "OK"
