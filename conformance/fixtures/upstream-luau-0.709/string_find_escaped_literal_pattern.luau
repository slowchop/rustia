assert(string.find("size (17) out of limits [1,16]", "%(17%) out of limits %[1,16%]") == 6)
assert(string.find("16-byte integer does not fit", "16%-byte integer") == 1)
assert(string.find("variable-length format", "variable%-length format") == 1)
assert(string.find("literal percent % value", "percent %% value") == 9)

return "OK"
