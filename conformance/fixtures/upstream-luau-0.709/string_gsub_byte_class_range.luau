local stripped, count = string.gsub("A\xC3\xA9\xE6\xBC\xA2", "[\x80-\xBF]", "")
assert(count == 3)
return stripped
