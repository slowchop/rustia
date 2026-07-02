-- Derived from Luau 0.709 tests/conformance/gc.luau weak-value table checks.
local a = {}
setmetatable(a, {__mode = "v"})

a[1] = string.rep("b", 21)
collectgarbage()
assert(a[1])
a[1] = nil

for i = 1, 4 do
    a[i] = {}
end
for i = 1, 4 do
    a[i .. "x"] = {}
end
for i = 1, 4 do
    local t = {}
    a[t] = t
end
for i = 1, 4 do
    a[i + 4] = i .. "x"
end

collectgarbage()

local count = 0
for k, v in pairs(a) do
    assert(k == v or k - 4 .. "x" == v)
    count += 1
end

return count
