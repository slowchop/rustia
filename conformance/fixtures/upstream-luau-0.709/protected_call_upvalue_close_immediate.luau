-- Derived from Luau 0.709 tests/conformance/pcall.luau immediate protected-call upvalue close cases.
local upclo

local function uptest(y)
    local a, b, c, d = 1, 2, 3, 4

    upclo = function()
        local t = table.pack("a", "b", "d", "e", "f", "g", "h", "i")
        assert(a == 1)
        assert(b == 2)
        assert(c == 3)
        assert(d == 4)
        a, b, c, d = table.unpack(t)
        return "ok"
    end

    if y then
        y()
    end

    error("oops")
end

do
    upclo = nil
    local ok = pcall(uptest, nil)
    assert(not ok)
    assert(upclo() == "ok")
end

do
    upclo = nil
    local ok = xpcall(uptest, function(_err)
    end, nil)
    assert(not ok)
    assert(upclo() == "ok")
end

do
    upclo = nil
    local ok, first = xpcall(uptest, function(_err)
        return "e", "e", "e", "e", "e", "e", "e", "e"
    end, nil)
    assert(not ok and first == "e")
    assert(upclo() == "ok")
end

do
    local ur
    upclo = nil
    local ok = xpcall(uptest, function(_err)
        ur = upclo()
    end, nil)
    assert(not ok and ur == "ok")
end

return "OK"
