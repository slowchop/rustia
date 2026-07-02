-- Derived from Luau 0.709 tests/conformance/closure.luau for numeric for
-- control variables captured by closures.

local a = {}
for i = 1, 10 do
    a[i] = {
        set = function(x)
            i = x
        end,
        get = function()
            return i
        end,
    }
    if i == 3 then
        break
    end
end

assert(a[4] == nil)
a[1].set(10)
assert(a[2].get() == 2)
a[2].set("a")
assert(a[3].get() == 3)
assert(a[2].get() == "a")
