-- Derived from Luau 0.709 tests/conformance/debug.luau debug.info argument errors.
local co = coroutine.create(function()
end)

local function qux(...)
    local ok, err = pcall(debug.info, ...)
    assert(not ok)
    return err
end

assert(qux():find("function or level expected"))
assert(qux(1):find("string expected"))
assert(qux(-1):find("level can't be negative"))
assert(qux(1, "?"):find("invalid option"))
assert(qux(1, "nn"):find("duplicate option"))
assert(qux(co):find("function or level expected"))
assert(qux(co, 1):find("string expected"))

return "OK"
