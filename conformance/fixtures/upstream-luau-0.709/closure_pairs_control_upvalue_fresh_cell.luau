-- Derived from Luau 0.709 tests/conformance/closure.luau for generic for
-- control variables captured by closures. The upstream test returns both
-- variables at once; this scalar fixture keeps the same cell-isolation checks
-- within rustia's current return-list surface.

local a = {}
for i, k in pairs({ "a", "b" }) do
    a[i] = {
        set_i = function(x)
            i = x
        end,
        get_i = function()
            return i
        end,
        set_k = function(x)
            k = x
        end,
        get_k = function()
            return k
        end,
    }
    if i == 2 then
        break
    end
end

a[1].set_i(10)
assert(a[2].get_i() == 2)
a[2].set_i("a")
assert(a[2].get_i() == "a")
a[1].set_k(20)
assert(a[2].get_k() == "b")
a[2].set_k("c")
assert(a[2].get_k() == "c")
