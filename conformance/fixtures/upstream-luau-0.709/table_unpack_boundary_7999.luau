-- Derived from Luau 0.709 tests/conformance/tables.luau large-unpack boundary check.
return select("#", table.unpack(table.create(7999, 1)))
