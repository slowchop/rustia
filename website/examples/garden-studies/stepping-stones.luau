local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local pos, stones = {}, {}
for i = 1, 100 do
    pos[i] = vector(math.random(950), math.random(540))
    if i <= 9 then stones[i] = pos[i] end
end
function update(time, mouse)
    if mouse then stones[1] = mouse end
    for i, p in pos do
        local home = stones[1 + (i % #stones)]
        local kick = vector(math.random(-18, 18), math.random(-18, 18))
        if math.random(20) == 1 then pos[i] += (home - p) * 0.16 + kick end
        sprite(pos[i], kinds[i % #kinds + 1])
    end
end
