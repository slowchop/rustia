-- PUC Lua 5.1.5 differential fixture: negative and fractional number keys
-- (hash part only — no array-part candidates), insert/delete churn.
-- Expectation captured offline from the pinned 5.1.5 oracle.
local seed = 9009
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for i = 1, 60 do
    t[-i - 0.25] = i
    if rnd(100) < 70 then
        t[i + 0.75] = i
    end
end
for i = 1, 60 do
    if rnd(100) < 25 then
        t[-i - 0.25] = nil
    end
end
local k, v = next(t)
while k ~= nil do
    emit(tostring(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
return table.concat(parts, ";")
