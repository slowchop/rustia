-- PUC Lua 5.1.5 differential fixture: table.insert / table.remove sequences
-- (append, positional insert, tail and positional remove), which resolve
-- through the PUC luaH_getn border. Expectation captured offline from the
-- pinned 5.1.5 oracle.
local seed = 11011
local function rnd(n)
    seed = (seed * 48271) % 2147483647
    return seed % n
end
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for i = 1, 120 do
    local c = rnd(100)
    if c < 45 then
        table.insert(t, i)
    elseif c < 65 then
        table.insert(t, rnd(#t + 1) + 1, i)
    elseif c < 85 then
        table.remove(t)
    else
        local n = #t
        if n > 0 then
            table.remove(t, rnd(n) + 1)
        end
    end
end
local k, v = next(t)
while k ~= nil do
    emit(tostring(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
emit("#=" .. tostring(#t))
return table.concat(parts, ";")
