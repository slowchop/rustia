-- Derived from Luau 0.709 tests/conformance/events.luau __call checks.
local unpack = table.unpack

local mt = {}
local a = setmetatable({}, mt)

mt.__call = function(t, ...)
    return t, {...}
end

local receiver, args = a(unpack({"a", 1}))
assert(receiver == a)
assert(args[1] == "a" and args[2] == 1 and args[3] == nil)

receiver, args = a()
assert(receiver == a)
assert(args[1] == nil)

local count
local chain_mt = {
    __call = function(t, ...)
        count = count + 1
        if t.f then
            return t.f(...)
        else
            return {...}
        end
    end,
}

local first = setmetatable({}, chain_mt)
local second = setmetatable({f = first}, chain_mt)
local third = setmetatable({f = second}, chain_mt)

count = 0
local result = third(3, 4, 5)
assert(count == 3 and result[1] == 3 and result[3] == 5)

return "OK"
