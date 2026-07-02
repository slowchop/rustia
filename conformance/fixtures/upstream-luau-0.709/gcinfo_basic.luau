-- Derived from Luau 0.709 tests/conformance/gc.luau legacy gcinfo usage.
local before = gcinfo()
collectgarbage()
local after = gcinfo()

if type(before) ~= "number" or type(after) ~= "number" then
    return "FAIL"
end

local bytes = gcinfo()
for _ = 1, 4 do
    local nbytes = gcinfo()
    if nbytes < bytes then
        return "OK"
    end
    bytes = nbytes
end

return "FAIL"
