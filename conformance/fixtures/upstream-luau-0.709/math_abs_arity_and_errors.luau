-- Derived from Luau 0.709 tests/conformance/basic.luau math.abs arity/error rows.
local touched = false
local function touch()
    touched = true
end

assert(math.abs(-5, touch()) == 5)
assert(touched)

local ok, err = pcall(function()
    return math.abs()
end)
assert(not ok and err:match("missing argument #1 to 'abs'"))

ok, err = pcall(function()
    return math.abs(nil)
end)
assert(not ok and err:match("invalid argument #1 to 'abs'"))

ok, err = pcall(function()
    return math.abs({})
end)
assert(not ok and err:match("invalid argument #1 to 'abs'"))

return "OK"
