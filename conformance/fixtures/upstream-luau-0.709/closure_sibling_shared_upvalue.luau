-- Derived from Luau 0.709 tests/conformance/closure.luau shared upvalue cases.

local x = 10

local function a()
    x += 1
    return x
end

local function b()
    x += 2
    return x
end

a()
return b()
