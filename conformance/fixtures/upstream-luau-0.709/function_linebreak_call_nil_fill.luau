-- Derived from Luau 0.709 tests/conformance/calls.luau multiline call argument adjustment checks.
function f(a, b, c)
    local d = "a"
    return {a, b, c, d}
end

local t = f(
    1, 2)

return t[1] == 1 and t[2] == 2 and t[3] == nil and t[4] == "a"
