-- PUC Lua 5.1.5 differential fixture: string-key-dominated table forced
-- through hash-part rehashes, with deletions and a second insert wave.
-- Expectation captured offline from the pinned 5.1.5 oracle.
local seed = 6006
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for i = 1, 140 do
    t["s" .. i] = i
end
for i = 1, 140 do
    if rnd(100) < 30 then
        t["s" .. i] = nil
    end
end
for i = 141, 200 do
    t["s" .. i] = i
end
local k, v = next(t)
while k ~= nil do
    emit(k .. "=" .. tostring(v))
    k, v = next(t, k)
end
return table.concat(parts, ";")
