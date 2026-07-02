-- Derived from Luau 0.709 tests/conformance/clear.luau post-clear array reuse.
local t = {}
for i = 1, 16 do
    t[i] = i
end
table.clear(t)
t[16] = true
local u = table.create(16)
u[16] = true
return #t
