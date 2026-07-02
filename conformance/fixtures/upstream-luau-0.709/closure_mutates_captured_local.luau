-- Derived from Luau 0.709 tests/conformance/closure.luau captured-local mutation cases.

local x = 1

local function inc()
    x += 1
    return x
end

inc()
return inc()
