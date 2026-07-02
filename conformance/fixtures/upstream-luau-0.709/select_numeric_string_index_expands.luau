-- Derived from Luau 0.709 tests/conformance/vararg.luau select("3", ...) checks.
return table.concat({select("3", 10, 20, 30)}, ",")
