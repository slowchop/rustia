-- Derived from Luau 0.709 tests/conformance/errors.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream assertion: rawset(t, nil, 2) errors with "table index is nil".
local t = {}
rawset(t, nil, 2)
