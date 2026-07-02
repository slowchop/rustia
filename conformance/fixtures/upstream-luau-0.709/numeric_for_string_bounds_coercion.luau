-- Derived from Luau 0.709 tests/conformance/iter.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers string-to-number coercion of numeric for start, limit, and step.
local count = 0
for _ = "10", "1", "-2" do
    count += 1
end
return count
