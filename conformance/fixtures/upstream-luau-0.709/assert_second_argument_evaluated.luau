-- Derived from Luau 0.709 tests/conformance/assert.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers multi-argument assert calls evaluating later arguments.
assert(true, assert(false))
