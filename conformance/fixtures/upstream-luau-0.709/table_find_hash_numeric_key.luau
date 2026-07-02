-- Derived from Luau 0.709 tests/conformance/tables.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers the upstream check that table.find sees numeric keys in the hash portion.
return table.find({[(2)] = true}, true, 2)
