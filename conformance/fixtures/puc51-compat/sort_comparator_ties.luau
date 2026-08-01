-- PUC Lua 5.1.5 differential fixture: table.sort with a comparator over
-- records with engineered duplicate sort keys; the id sequence pins the exact
-- auxsort tie order. Expectation captured offline from the pinned 5.1.5
-- oracle.
local seed = 14014
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local r = {}
for i = 1, 48 do
    r[i] = { id = i, k = rnd(6) }
end
table.sort(r, function(x, y)
    return x.k < y.k
end)
for i = 1, 48 do
    emit(r[i].id .. ":" .. r[i].k)
end
return table.concat(parts, ";")
