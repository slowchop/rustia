-- Derived from Luau 0.709 tests/conformance/events.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream rawlen table and string checks.
assert(rawlen({1}) == 1)
assert(rawlen("foo") == 3)
