local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local seeds = {}
for i = 1, 100 do
    seeds[i] = vector(math.random(980), math.random(560))
end
function update(time, mouse)
    for i, p in seeds do
        local bank = if i % 2 == 0 then 130 else 430
        local merge = math.clamp((p.x - 130) / 650, 0, 1)
        local stream = bank + (280 - bank) * merge + math.noise(p.x / 210, i % 2) * 65
        local away = if mouse then p - mouse else vector.zero
        local wake = math.max(0, 1 - vector.magnitude(away) / 100)
        local offset = vector(0.45 + i % 4 * 0.12, (stream - p.y) * 0.008 + away.y * wake * 0.022)
        seeds[i] = if p.x > 1012 then vector(-32, bank + i % 55) else p + offset
        sprite(seeds[i], kinds[i % #kinds + 1])
    end
end
