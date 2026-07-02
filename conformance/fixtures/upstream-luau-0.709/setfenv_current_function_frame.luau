-- Derived from Luau 0.709 tests/conformance/tables.luau current-frame setfenv checks.
local function foo()
    local getfenv, setfenv, assert, next = getfenv, setfenv, assert, next
    local n = { gl1 = 3 }
    setfenv(foo, n)
    assert(getfenv(foo) == getfenv(1))
    assert(getfenv(foo) == n)
    assert(print == nil and gl1 == 3)
    gl1 = nil
    gl = 1
    assert(n.gl == 1 and next(n, "gl") == nil)
    return "OK"
end

return foo()
