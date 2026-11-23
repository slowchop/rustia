local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local pos, speed = {}, {}
for i = 1, 100 do
    pos[i] = vector(math.random(980), math.random(560))
    speed[i] = vector(0, 0)
end
function update(time, mouse)
    for i, p in pos do
        local away = if mouse then p - mouse else vector.zero
        local touch = math.max(0, 1 - vector.magnitude(away) / 110)
        local wind = vector(0.22 + i % 4 * 0.07, math.noise(time * 0.06, i * 0.09))
        local nextSpeed = wind + away * touch * 0.018
        speed[i] += (nextSpeed - speed[i]) * 0.035
        local next = p + speed[i]
        pos[i] = vector((next.x + 40) % 1060 - 40, (next.y + 40) % 640 - 40)
        sprite(pos[i], kinds[i % #kinds + 1])
    end
end
