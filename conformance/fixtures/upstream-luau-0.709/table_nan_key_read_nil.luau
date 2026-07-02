-- Derived from Luau 0.709 tests/conformance/math.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream assertions that table reads using NaN keys return nil.
local NaN
NaN = 10e500 - 10e400
local a = {}
assert(a[NaN] == nil)
a[1] = 1
assert(a[NaN] == nil)
