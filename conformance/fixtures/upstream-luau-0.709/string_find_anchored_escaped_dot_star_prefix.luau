-- Derived from Luau 0.709 tests/conformance/errors.luau syntax-error message prefix check.
local msg = [[ [string "break label"]:1: syntax error ]]
msg = string.sub(msg, 2)
assert(string.find(msg, [[^%[string ".*"%]:]]) == 1)
assert(string.find(msg, [[^%[string ".*"%]:]], 2) == nil)
assert(string.find("prefix " .. msg, [[^%[string ".*"%]:]]) == nil)
return "OK"
