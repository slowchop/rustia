-- PUC Lua 5.1.5 differential fixture: deletion of the current key during
-- next() traversal (the only deletion PUC's contract allows mid-traversal),
-- then resurrection of tombstoned keys. Expectation captured offline from the
-- pinned 5.1.5 oracle.
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
local t = {}
for i = 1, 40 do
    t[i] = i
end
for i = 1, 48 do
    t["s" .. i] = i
end
t[true] = "T"
t[false] = "F"
local k, v = next(t)
while k ~= nil do
    local cur = k
    local del = type(cur) == "string" or (type(cur) == "number" and cur % 2 == 0)
    if del then
        emit("del:" .. keystr(cur))
        t[cur] = nil
    else
        emit("keep:" .. keystr(cur))
    end
    k, v = next(t, cur)
end
t["s1"] = 9001
t[2] = 9002
local k2, v2 = next(t)
while k2 ~= nil do
    emit("live:" .. keystr(k2) .. "=" .. tostring(v2))
    k2, v2 = next(t, k2)
end
emit("#=" .. tostring(#t))
return table.concat(parts, ";")
