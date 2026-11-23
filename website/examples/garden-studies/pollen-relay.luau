local flowers, bee = {}, vector(490, 280)
local stop = 1
for i = 1, 64 do
    flowers[i] = vector(math.random(24, 950), math.random(24, 530))
end
function update(time, mouse)
    local target = flowers[stop]
    if mouse and vector.magnitude(mouse - bee) < 90 then target = mouse end
    local gap = target - bee
    bee += gap / math.max(1, vector.magnitude(gap) / 3)
    if vector.magnitude(flowers[stop] - bee) < 6 then stop = math.random(#flowers) end
    for i, p in flowers do
        sprite(p, if i == stop then Sprite.Ring else Sprite.Flower)
    end
    sprite(bee, Sprite.Bud)
end
