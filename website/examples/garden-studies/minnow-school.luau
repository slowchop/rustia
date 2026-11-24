local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local fish, speed = {}, {}
for i = 1, 90 do
    fish[i] = vector(math.random(980), math.random(560))
    speed[i] = vector(0.5, 0)
end
function update(time, mouse)
    for i, p in fish do
        local friend = 1 + i % #fish
        local gap = fish[friend] - p
        local steer = gap * (1 - 42 / math.max(1, vector.magnitude(gap)))
        if mouse and vector.magnitude(mouse - p) < 130 then steer += (mouse - p) * 0.6 end
        local eddy = vector(math.noise(i, time * 0.12), math.noise(time * 0.12, i))
        speed[i] = speed[i] * 0.93 + speed[friend] * 0.04 + steer * 0.0005 + eddy * 0.045
        fish[i] = vector.clamp(p + speed[i], vector(15, 15), vector(965, 545))
        sprite(fish[i], kinds[i % #kinds + 1])
    end
end
