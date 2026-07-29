-- Derived from Luau 0.709 tests/conformance/gc.luau legacy gcinfo usage.
-- The exact byte count and collector phase are implementation-defined, so
-- create explicit unreachable pressure instead of requiring polling alone to
-- advance the reference collector.
local garbage = {}
for i = 1, 256 do
    garbage[i] = { i, i + 1, i + 2 }
end
local before = gcinfo()
garbage = nil
collectgarbage()
local after = gcinfo()

if type(before) ~= "number" or type(after) ~= "number" or after > before then
    return "FAIL"
end

return "OK"
