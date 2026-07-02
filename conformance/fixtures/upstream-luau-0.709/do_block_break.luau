-- Derived from Luau 0.709 tests/conformance/closure.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers a scoped do block containing break inside an enclosing loop.
local a = 0
while 1 do
    do
        break
    end
    a = 1
end
return a
