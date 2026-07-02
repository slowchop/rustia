-- Derived from Luau 0.709 tests/conformance/gc.luau weak-key table checks.
local a = {}
setmetatable(a, {__mode = "k"})

for i = 1, 4 do
    a[{}] = i
end
for i = 1, 4 do
    local t = {}
    a[t] = t
end
for i = 1, 4 do
    a[i] = i
end
for i = 1, 4 do
    local s = string.rep("@", i)
    a[s] = s .. "#"
end

collectgarbage()

local count = 0
for k, v in pairs(a) do
    assert(k == v or k .. "#" == v)
    count += 1
end

return count
