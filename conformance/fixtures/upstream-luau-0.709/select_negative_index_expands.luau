-- Derived from Luau 0.709 tests/conformance/vararg.luau select(-2, ...) checks.
return table.concat({select(-2, 10, 20, 30)}, ",")
