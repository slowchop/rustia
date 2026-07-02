-- Derived from Luau 0.709 tests/conformance/closure.luau multi-level closure cases.

local w = 1

local function f(x)
    return function(y)
        return function(z)
            return w + x + y + z
        end
    end
end

return f(10)(20)(30)
