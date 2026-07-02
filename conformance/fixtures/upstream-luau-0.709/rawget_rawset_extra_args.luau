-- Derived from Luau 0.709 tests/conformance/events.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream rawget/rawset extra-argument checks.
local a = {}
rawset(a, "x", 1, 2, 3)
assert(a.x == 1 and rawget(a, "x", 3) == 1)
