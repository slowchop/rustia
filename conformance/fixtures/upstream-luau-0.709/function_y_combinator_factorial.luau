-- Derived from Luau 0.709 tests/conformance/calls.luau fixed-point combinator factorial check.
local Y = function(le)
    local function a(f)
        return le(function(x)
            return f(f)(x)
        end)
    end
    return a(a)
end

local F = function(f)
    return function(n)
        if n == 0 then
            return 1
        end
        return n * f(n - 1)
    end
end

return Y(F)(5)
