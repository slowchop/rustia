-- PUC Lua 5.1.5 differential fixture: %.14g formatting of deterministic
-- PRNG-generated doubles (scaled by powers of two and ten) through tostring
-- and table.concat. Expectation captured offline from the pinned 5.1.5
-- oracle.
local seed = 12012
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
for i = 1, 60 do
    local m = rnd(2000000000) + 1
    local v = m / 2 ^ rnd(60)
    emit(tostring(v))
    local k = rnd(21) - 6
    local w = m * 10 ^ k
    emit(table.concat({ w }, ","))
end
return table.concat(parts, ";")
