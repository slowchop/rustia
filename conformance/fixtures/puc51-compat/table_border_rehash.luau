-- PUC Lua 5.1.5 differential fixture: `#` border evolution while sequential
-- inserts cross array-part rehash boundaries, then the full next() trace.
-- Expectation captured offline from the pinned 5.1.5 oracle.
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local t = {}
for i = 1, 130 do
    t[i] = i
    emit("b" .. i .. "=" .. tostring(#t))
end
local k, v = next(t)
while k ~= nil do
    emit(tostring(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
return table.concat(parts, ";")
