-- Derived from Luau 0.709 tests/conformance/calls.luau nested closure parameter capture check.
local function g(z)
    local function f(a, b, c, d)
        return function(x, y)
            return a + b + c + d + a + x + y + z
        end
    end

    return f(z, z + 1, z + 2, z + 3)
end

local f = g(10)
return f(9, 16)
