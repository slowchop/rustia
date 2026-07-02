-- Derived from Luau 0.709 tests/conformance/closure.luau tail-call upvalue close case.
local function t()
    local function c(a, b)
        assert(a == "test" and b == "OK")
    end

    local function v(f, ...)
        c("test", f() ~= 1 and "FAILED" or "OK")
    end

    local x = 1
    return v(function()
        return x
    end)
end

t()
return "OK"
