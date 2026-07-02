-- Derived from Luau 0.709 tests/conformance/debug.luau traceback named-frame cases.
function foo(...)
    return debug.traceback(...)
end

assert(foo():find("foo") > 0)
assert(foo("hello"):find("hello") > 0)
assert(foo("hello"):find("foo") > 0)
assert(foo("hello", 2):find("hello") > 0)
assert(foo("hello", 2):find("foo") == nil)

return "OK"
