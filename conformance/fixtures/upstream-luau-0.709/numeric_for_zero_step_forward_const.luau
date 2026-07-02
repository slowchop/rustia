-- Derived from Luau 0.709 tests/conformance/basic.luau at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers zero-step numeric for treating step 0 as backward iteration, constant step form.
local c = 0
for i = 1, 10, 0 do
    c += 1
    if c > 10 then
        break
    end
end
return c
