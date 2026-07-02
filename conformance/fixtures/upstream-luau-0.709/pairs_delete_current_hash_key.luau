-- Derived from Luau 0.709 tests/conformance/tables.luau erase-during-pairs checks.
local t = {
    [{ 1 }] = 1,
    [{ 2 }] = 2,
    [string.rep("x ", 4)] = 3,
    [100.3] = 4,
    [4] = 5,
}

local n = 0
for k, v in pairs(t) do
    n = n + 1
    assert(t[k] == v)
    t[k] = nil
    collectgarbage()
    assert(t[k] == nil)
end

return n
