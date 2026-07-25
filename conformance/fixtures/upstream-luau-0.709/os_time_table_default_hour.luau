-- Derived from Luau 0.709 VM/src/loslib.cpp at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- os_time reads the date-table hour field with getfield(L, "hour", 12), so a
-- missing hour defaults to 12 (noon), not 0 (midnight).
local implicit = os.time({ year = 2000, month = 6, day = 10 })
local noon = os.time({ year = 2000, month = 6, day = 10, hour = 12 })
local midnight = os.time({ year = 2000, month = 6, day = 10, hour = 0 })
assert(implicit == noon, "os.time date table without hour must match hour = 12")
assert(implicit ~= midnight, "os.time date table without hour must differ from hour = 0")
return implicit == noon and implicit ~= midnight
