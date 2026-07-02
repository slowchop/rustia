-- Derived from Luau 0.709 tests/conformance/assert.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream assertion: assert(ecall(function() assert(false) end) == "assertion failed!").
assert(false)
