local hits = 0
local t = setmetatable({ 1, nil, 3 }, {
    __index = function(_, key)
        hits = hits + 1
        return key * 10
    end,
    __len = function()
        return 3
    end,
})

assert(t[2] == 20)
assert(hits == 1)
hits = 0

local sum = 0
for key, value in ipairs(t) do
    sum = sum + key * 10 + value
end
assert(sum == 11)
assert(hits == 0)

local inext, state, control = ipairs(t)
assert(state == t and control == 0)
local key, value = inext(state, 1)
assert(key == nil and value == nil)
assert(hits == 0)

local proxy = setmetatable({}, {
    __index = function(_, key)
        hits = hits + 1
        return key * 10
    end,
})

local iterated = false
for _ in ipairs(proxy) do
    iterated = true
end

assert(iterated == false)
assert(hits == 0)

return "OK"
