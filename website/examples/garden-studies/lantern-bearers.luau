local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local lights, goals = {}, {}
for i = 1, 60 do
    lights[i] = vector(math.random(980), math.random(560))
    goals[i] = lights[i]
end
function update(time, mouse)
    for i, p in lights do
        if vector.magnitude(goals[i] - p) < 12 then
            goals[i] = vector(math.random(950), math.random(540))
        end
        local gap = goals[i] - p
        if mouse and vector.magnitude(mouse - p) < 125 then gap = mouse - p end
        lights[i] += gap / math.max(140, vector.magnitude(gap) / 0.55)
        sprite(lights[i], kinds[i % #kinds + 1])
    end
end
