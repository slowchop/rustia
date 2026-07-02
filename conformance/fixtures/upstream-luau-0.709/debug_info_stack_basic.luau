-- Derived from Luau 0.709 tests/conformance/debug.luau debug.info active stack cases.
local function baz(...)
    return debug.info(...)
end

assert(baz(0, "n") == "info")
assert(baz(1, "n") == "baz")
assert(baz(2, "n") == "")
assert(baz(3, "n") == nil)
assert(baz(0, "s") == "[C]")
assert(baz(1, "s") == "debug_info_stack_basic.luau")
assert(baz(0, "l") == -1)
assert(baz(1, "l") > 0)
assert(baz(0, "f") == debug.info)
assert(baz(1, "f") == baz)
assert(baz(0, "a") == 0)
assert(baz(1, "a") == 0)

local function quux(...)
    return { debug.info(...) }
end

local info = quux(1, "nlsf")
assert(#info == 4)
assert(info[1] == "quux")
assert(info[2] > 0)
assert(info[3] == "debug_info_stack_basic.luau")
assert(info[4] == quux)

return "OK"
