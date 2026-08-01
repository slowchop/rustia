-- PUC Lua 5.1.5 differential fixture: table constructors — positional runs
-- (incl. the 17-element int2fb round-up and a 60-element multi-flush), keyed
-- fields, duplicate positional/keyed writes, call and vararg tails, nesting.
-- Expectation captured offline from the pinned 5.1.5 oracle.
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
local a = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17 }
emit("a#=" .. tostring(#a))
local b = { x = 1, y = 2, z = 3 }
trace(b, "b:")
local c = { 1, 2, 3, x = "a", y = "b", [10] = 10, [20] = 20 }
emit("c#=" .. tostring(#c))
trace(c, "c:")
-- Duplicate keyed field: PUC and rustia both evaluate keyed fields in source
-- order, so x ends "uno". (Overlapping positional/keyed writes such as
-- `{ 1, 2, 3, [1] = "one", 4 }` diverge rustia-wide: rustia evaluates
-- constructor fields in source order, while PUC flushes positional items
-- after keyed ones — see docs/accepted divergences.)
local d = { 1, 2, 3, x = "one", x = "uno", 4 }
emit("d#=" .. tostring(#d))
trace(d, "d:")
local function g()
    return 1, 2, 3
end
local e = { 0, g() }
emit("e#=" .. tostring(#e))
trace(e, "e:")
local f = { g(), 9 }
emit("f#=" .. tostring(#f))
trace(f, "f:")
local function h(...)
    return { ... }
end
local w = h(7, 8, 9)
emit("w#=" .. tostring(#w))
trace(w, "w:")
local big = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60,
}
emit("big#=" .. tostring(#big))
local nested = { { 1, 2 }, { 3, 4, 5 } }
emit("n#=" .. tostring(#nested) .. "," .. tostring(#nested[1]) .. "," .. tostring(#nested[2]))
return table.concat(parts, ";")
