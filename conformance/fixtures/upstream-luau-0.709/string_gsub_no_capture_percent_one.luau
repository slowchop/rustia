local result, count = string.gsub("abc", "[a-c]", "%1%0")
assert(count == 3)
return result
