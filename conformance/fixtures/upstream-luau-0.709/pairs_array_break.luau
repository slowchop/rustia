-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers break from generic for over pairs.
local a = 1
for _ in pairs({1, 2, 3}) do
    a = a * 2
    if a == 4 then
        break
    end
end
return a
