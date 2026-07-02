local a = string.gsub("(..*) %2", "^(^?)", "%1()", 1)
local b = string.gsub("^abc$", "^(^?)", "%1()", 1)
local c = string.gsub(a, "($?)$", "()%1", 1)
local d = string.gsub(b, "($?)$", "()%1", 1)

assert(a == "()(..*) %2")
assert(b == "^()abc$")
assert(c == "()(..*) %2()")
assert(d == "^()abc()$")

local result, count = string.gsub("a b c", "[abc]", { a = "A", c = false })
assert(result == "A b c" and count == 3)

return c
