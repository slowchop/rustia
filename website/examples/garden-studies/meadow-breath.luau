local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local roots, sway = {}, {}
for i = 1, 100 do
    roots[i] = vector(math.random(980), math.random(560))
    sway[i] = vector(0, 0)
end
function update(time, mouse)
    for i, root in roots do
        local gust = math.noise(root.x * 0.003 - time * 0.15, root.y * 0.004)
        local away = if mouse then root - mouse else vector.zero
        local nearby = math.max(0, 1 - vector.magnitude(away) / 130)
        local bend = vector(gust * 38, gust * 10) + away * nearby * 0.6
        sway[i] += (bend - sway[i]) * 0.045
        sprite(root + sway[i], kinds[i % #kinds + 1], sway[i].x * 0.5)
    end
end
