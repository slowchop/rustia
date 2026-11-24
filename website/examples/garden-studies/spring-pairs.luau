local pos, speed = {}, {}
for i = 1, 80 do
    pos[i] = vector(math.random(950), math.random(540))
    speed[i] = vector(0, 0)
end
function update(time, mouse)
    for i, p in pos do
        local buddy = pos[if i % 2 == 0 then i - 1 else i + 1]
        local gap = buddy - p
        local stretch = 1 - 52 / math.max(1, vector.magnitude(gap))
        local kick = vector(math.random() - 0.5, math.random() - 0.5)
        local push = if mouse and vector.magnitude(p - mouse) < 95 then (p - mouse) * 0.02 else kick
        speed[i] = speed[i] * 0.94 + gap * stretch * 0.018 + push
        pos[i] = vector.clamp(p + speed[i], vector(12, 12), vector(970, 550))
        sprite(pos[i], if i % 2 == 0 then Sprite.Flower else Sprite.Bud)
    end
end
