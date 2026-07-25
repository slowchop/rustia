-- Derived from Luau 0.709 VM/src/loslib.cpp at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- os_date fills "*t" tables with setboolfield(L, "isdst", tm_isdst); gmtime_r
-- reports tm_isdst 0 for UTC, so UTC date tables carry isdst = false.
local parts = os.date("!*t", 0)
assert(type(parts.isdst) == "boolean", 'os.date("!*t") must set a boolean isdst field')
assert(parts.isdst == false, 'os.date("!*t") isdst must be false for UTC')
return type(parts.isdst) == "boolean" and parts.isdst == false
