-- Derived from Luau 0.709 VM/src/ltable.cpp numeric-key normalization at
-- b968ef742741bb2b703afc3b3c53f06608c87481.
-- Covers canonicalizing -0 and 0 to one raw table key.
local t = {}
t[-0] = "negative"

local read_from_zero = t[0] == "negative"
t[0] = "zero"
local read_from_negative_zero = t[-0] == "zero"

local count = 0
local observed_key = nil
for key in pairs(t) do
    count += 1
    observed_key = key
end

return read_from_zero
    and read_from_negative_zero
    and count == 1
    and tostring(observed_key) == "0"
