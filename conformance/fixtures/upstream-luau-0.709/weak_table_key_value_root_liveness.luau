-- Derived from Luau 0.709 tests/conformance/gc.luau weak key/value root liveness checks.
local lim = 100
local a = {}
setmetatable(a, {__mode = "vk"})

local x, y, z = {}, {}, {}
a[1], a[2], a[3] = x, y, z
a[string.rep("$", 11)] = string.rep("$", 11)

for i = 4, lim do
    a[i] = {}
end
for i = 1, lim do
    a[{}] = i
end
for i = 1, lim do
    local t = {}
    a[t] = t
end

collectgarbage()

local count = 0
for k, v in pairs(a) do
    assert((k == 1 and v == x) or (k == 2 and v == y) or (k == 3 and v == z) or k == v)
    count += 1
end

return count
