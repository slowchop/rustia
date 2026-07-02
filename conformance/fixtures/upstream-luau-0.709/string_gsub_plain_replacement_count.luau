-- Derived from Luau 0.709 tests/conformance/literals.luau string.gsub line-ending rewrite.
local rewritten, count = string.gsub("a\nb\n", "\n", "|")
if count == 2 then
    return rewritten
end
return "BAD"
