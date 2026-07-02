assert(("wrong number of arguments"):match("wrong number of arguments") == "wrong number of arguments")
assert(string.match("abc", "b") == "b")
assert(string.match("abc", ".b.") == "abc")
assert(string.match("abc", "b", "2") == "b")
assert(string.match("abc", "b", 2.9) == "b")
assert(string.match("abc", "a", nil) == "a")
assert(string.match("abc", "z") == nil)

return "OK"
