-- PUC Lua 5.1.5 differential fixture: table.sort over engineered array shapes
-- (ascending, descending, organ-pipe, all-equal, sawtooth with duplicates).
-- Expectation captured offline from the pinned 5.1.5 oracle.
local parts = {}
local function emit(s)
    parts[#parts + 1] = s
end
local function emitarr(tag, a)
    table.sort(a)
    emit(tag .. "=" .. table.concat(a, ","))
end
local asc = {}
for i = 1, 32 do
    asc[i] = i
end
emitarr("asc", asc)
local desc = {}
for i = 1, 32 do
    desc[i] = 33 - i
end
emitarr("desc", desc)
local organ = {}
for i = 1, 16 do
    organ[i] = i
    organ[33 - i] = i
end
emitarr("organ", organ)
local same = {}
for i = 1, 24 do
    same[i] = 5
end
emitarr("same", same)
local saw = {}
for i = 1, 40 do
    saw[i] = i % 4
end
emitarr("saw", saw)
return table.concat(parts, ";")
