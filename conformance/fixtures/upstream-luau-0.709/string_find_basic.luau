-- Derived from Luau 0.709 tests/conformance/strings.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
a, b = string.find("123456789", "345")
assert(string.sub("123456789", a, b) == "345")
return string.find("123456789", "345")
