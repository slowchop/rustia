-- PUC Lua 5.1.5 differential fixture: next() walks from the start and from an
-- explicit middle key, the "invalid key to 'next'" error, and next() on an
-- empty table. Expectation captured offline from the pinned 5.1.5 oracle.
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
local t = { 10, 20, 30, x = "a", y = "b" }
local k, v = next(t)
while k ~= nil do
    emit(keystr(k) .. "=" .. tostring(v))
    k, v = next(t, k)
end
local mk, mv = next(t, 1)
emit("from1=" .. keystr(mk) .. "=" .. tostring(mv))
local ok, err = pcall(next, t, "missing")
emit("err=" .. tostring(ok) .. ":" .. tostring(string.find(tostring(err), "invalid key", 1, true) ~= nil))
local empty = {}
emit("empty=" .. tostring(next(empty)))
return table.concat(parts, ";")
