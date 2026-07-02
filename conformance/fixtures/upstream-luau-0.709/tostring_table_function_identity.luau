-- Derived from Luau 0.709 tests/conformance/strings.luau tostring identity checks.
assert(string.find(tostring({}), "table:"))
assert(string.find(tostring(print), "function:"))

return "OK"
