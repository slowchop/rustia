-- PUC Lua 5.1.5 differential fixture: mixed scalar keys under insert/delete churn.
-- Expectation captured offline from the pinned 5.1.5 oracle; see conformance/README.md.
-- Deterministic in-guest PRNG (Park-Miller MINSTD: state * 48271 < 2^53, exact in doubles).
local seed = 1001
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local function keystr(k)
    local ty = type(k)
    if ty == "number" then
        return "n:" .. tostring(k)
    end
    if ty == "string" then
        return "s:" .. k
    end
    return "b:" .. tostring(k)
end
local function pickkey()
    local c = rnd(100)
    if c < 40 then
        return rnd(89) - 8
    end
    if c < 55 then
        return rnd(21) + 0.5
    end
    if c < 90 then
        return "k" .. rnd(48)
    end
    return rnd(2) == 1
end
local t = {}
for i = 1, 400 do
    local k = pickkey()
    if rnd(100) < 65 then
        t[k] = i
    else
        t[k] = nil
    end
end
local k, v = next(t)
while k ~= nil do
    emit(keystr(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
emit("#=" .. tostring(#t))
return table.concat(parts, ";")
