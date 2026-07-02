-- Derived from Luau 0.709 tests/conformance/errors.luau error level lineerror cases.
local function lineerror(s)
    local ok, msg = pcall(loadstring(s))
    local line = string.match(msg, ":(%d+):")
    return line and line + 0
end

local p = [[
function g() f() end
function f(x) error('a', X) end
g()
]]

X = 3
assert(lineerror(p) == 3)
X = 0
assert(lineerror(p) == nil)
X = 1
assert(lineerror(p) == 2)
X = 2
assert(lineerror(p) == 1)
return "OK"
