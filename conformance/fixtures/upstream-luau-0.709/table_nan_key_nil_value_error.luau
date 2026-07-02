-- Derived from Luau 0.709 tests/conformance/errors.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream assertion: t[0/0] = nil errors with "table index is NaN".
local t = {}
t[0/0] = nil
