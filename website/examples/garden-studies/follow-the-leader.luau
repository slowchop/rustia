local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local pos, goals = {}, {}
for i = 1, 100 do
    pos[i] = vector(math.random(950), math.random(540))
end
function update(time, mouse)
    for i, p in pos do
        local leader = i % 10 == 1
        local target = if leader then goals[i] or p else pos[i - 1]
        if leader and vector.magnitude(target - p) < 20 then
            target = vector(math.random(950), math.random(540))
            goals[i] = target
        end
        if leader and mouse and vector.magnitude(mouse - p) < 150 then target = mouse end
        pos[i] += (target - p) * (if leader then 0.012 else 0.07)
        sprite(pos[i], kinds[i % #kinds + 1])
    end
end
