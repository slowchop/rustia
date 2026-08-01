-- PUC Lua 5.1.5 differential fixture: jumping-k integer writes (powers of two)
-- over a sparse array, hole filling, and deletions straddling the border.
-- Expectation captured offline from the pinned 5.1.5 oracle.
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for j = 0, 10 do
    t[2 ^ j] = j
    emit("j" .. j .. "=" .. tostring(#t))
end
t[3] = 33
t[5] = 55
t[7] = 77
t[100] = 100
t[2] = nil
t[64] = nil
emit("#=" .. tostring(#t))
local k, v = next(t)
while k ~= nil do
    emit(tostring(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
return table.concat(parts, ";")
