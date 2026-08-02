-- PUC Lua 5.1.5 differential fixture: boolean keys alongside number and
-- string keys (5.1 keeps t[1] and t["1"] distinct), boolean tombstone and
-- resurrection. Expectation captured offline from the pinned 5.1.5 oracle.
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
local function trace(t, tag)
    local k, v = next(t)
    while k ~= nil do
        emit(tag .. keystr(k) .. "=" .. tostring(v))
        k, v = next(t, k)
    end
end
local t = {}
t[true] = "T"
t[false] = "F"
t[1] = "one"
t["1"] = "str-one"
t["true"] = "str-true"
t[0] = "zero"
t[-1] = "neg"
trace(t, "a:")
t[true] = nil
t[false] = nil
t[true] = "T2"
trace(t, "b:")
emit("#=" .. tostring(#t))
return table.concat(parts, ";")
