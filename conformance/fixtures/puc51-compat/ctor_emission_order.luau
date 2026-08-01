-- PUC Lua 5.1.5 differential fixture: constructor write order — keyed fields
-- write immediately at their source position while positional values flush in
-- LFIELDS_PER_FLUSH (50) batches plus a final SETLIST, so a later positional
-- batch overwrites an earlier keyed write to the same index. Reference Luau
-- (and rustia's other profiles) keep source order instead; these expectations
-- pin the PUC behavior, captured offline from the pinned 5.1.5 oracle.
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local a = { 1, 2, 3, [1] = "one", 4 }
emit("a=" .. tostring(a[1]) .. "," .. tostring(a[4]))
local b = { 1, [1] = "one" }
emit("b=" .. tostring(b[1]))
local c = { [1] = "one", 1 }
emit("c=" .. tostring(c[1]))
local d = { 1, x = 5, [1] = 9, 2 }
emit("d=" .. tostring(d[1]) .. "," .. tostring(d.x) .. "," .. tostring(d[2]))
local function g()
    return 10, 20
end
local e = { 1, [2] = "k", g() }
emit("e=" .. tostring(e[1]) .. "," .. tostring(e[2]) .. "," .. tostring(e[3]))
local function v(...)
    local t = { 1, [2] = "k", ... }
    return tostring(t[2]) .. "," .. tostring(t[3])
end
emit("v=" .. v(10, 20))
-- Field evaluation order stays source order even though the positional write
-- wins the slot.
local log = {}
local function mark(x)
    log[#log + 1] = x
    return x
end
local m = { mark(1), [1] = "k", mark(2) }
emit("m=" .. table.concat(log, ",") .. "/" .. tostring(m[1]))
-- The 50-item flush batch is observable: a keyed write inside the first batch
-- is overwritten by the flush, while one after the batch survives.
local inside = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    21, 22, 23, 24, 25,
    [1] = "x",
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
    46, 47, 48, 49, 50,
}
emit("inside=" .. tostring(inside[1]) .. "/" .. tostring(inside[50]))
local after = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    [1] = "x",
    51,
}
emit("after=" .. tostring(after[1]) .. "/" .. tostring(after[51]))
return table.concat(parts, ";")
