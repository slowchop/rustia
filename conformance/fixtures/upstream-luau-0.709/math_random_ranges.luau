math.randomseed(0)

local seen_low = false
local seen_high = false
for _ = 1, 10000 do
    local value = math.random()
    assert(0 <= value and value < 1)
    seen_low = seen_low or value < 0.001
    seen_high = seen_high or value > 0.999
    if seen_low and seen_high then
        break
    end
end
assert(seen_low and seen_high)

for _ = 1, 10 do
    local value = math.random(5)
    assert(1 <= value and value <= 5)
end

seen_low = false
seen_high = false
for _ = 1, 10000 do
    local value = math.random(-10, 0)
    assert(-10 <= value and value <= 0)
    seen_low = seen_low or value == -10
    seen_high = seen_high or value == 0
    if seen_low and seen_high then
        break
    end
end
assert(seen_low and seen_high)

local ok, err = pcall(math.random, 1, 2, 3)
assert(not ok and string.find(err, "wrong number of arguments", 1, true))
assert(select("#", math.randomseed(0)) == 0)
assert(select("#", math.randomseed(1, 2)) == 0)

return "OK"
