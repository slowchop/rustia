-- PUC Lua 5.1.5 differential fixture: sequential inserts across rehash
-- boundaries, ~60% deletion (tombstones), then a second insert wave reusing
-- freed slots. Expectation captured offline from the pinned 5.1.5 oracle.
local seed = 2002
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for i = 1, 240 do
    t[i] = i
end
for i = 1, 240 do
    if rnd(100) < 60 then
        t[i] = nil
    end
end
for i = 241, 300 do
    t[i] = i
    t["c" .. i] = i
end
local k, v = next(t)
while k ~= nil do
    local ty = type(k)
    if ty == "number" then
        emit("n:" .. tostring(k) .. "=" .. tostring(v))
    else
        emit("s:" .. k .. "=" .. tostring(v))
    end
    k, v = next(t, k)
end
emit("#=" .. tostring(#t))
return table.concat(parts, ";")
