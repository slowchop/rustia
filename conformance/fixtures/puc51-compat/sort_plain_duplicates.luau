-- PUC Lua 5.1.5 differential fixture: table.sort with the default less-than
-- order over arrays with engineered duplicate keys; the exact post-sort order
-- (PUC auxsort is not stable, so duplicates pin the exact partition sequence).
-- Expectation captured offline from the pinned 5.1.5 oracle.
local seed = 13013
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local a = {}
for i = 1, 64 do
    a[i] = rnd(8)
end
table.sort(a)
emit(table.concat(a, ","))
local b = {}
for i = 1, 48 do
    b[i] = (i * 7) % 5
end
table.sort(b)
emit(table.concat(b, ","))
return table.concat(parts, ";")
