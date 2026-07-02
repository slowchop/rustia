-- Derived from Luau 0.709 tests/conformance/closure.luau weak-table GC polling.
local marker = {[1] = {}}
setmetatable(marker, {__mode = "kv"})

local polls = 0
while marker[1] do
    local _garbage = tostring(polls) .. tostring(polls)
    polls += 1
    if polls > 100000 then
        return "timeout"
    end
end

return "OK"
