-- Derived from Luau 0.709 tests/conformance/vararg.luau oneless(oneless(oneless(...))) checks.
local function oneless(a, ...)
    return ...
end

local function f(a, ...)
    local b, c, d = ...
    return a, b, c, d, oneless(oneless(oneless(...)))
end

local a, b, c, d, e = f(5, 4, 3, 2, 1)
return (a or 0) * 10000 + (b or 0) * 1000 + (c or 0) * 100 + (d or 0) * 10 + (e or 0)
