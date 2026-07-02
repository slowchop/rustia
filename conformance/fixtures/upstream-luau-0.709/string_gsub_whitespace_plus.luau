-- Derived from Luau 0.709 tests/conformance/constructs.luau string.gsub("%s+", "\n") rewrite.
local rewritten, count = string.gsub("a  b\tc\n", "%s+", "|")

assert(count == 3)

return rewritten
