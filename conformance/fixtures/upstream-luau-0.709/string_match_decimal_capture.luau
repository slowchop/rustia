-- Derived from Luau 0.709 tests/conformance/errors.luau stack line parsing.
local line = [[src:164: stack frame]]
local curr = string.match(line, ":(%d+):")
assert(curr == "164")
assert(string.match("no line number here", ":(%d+):") == nil)
return curr
