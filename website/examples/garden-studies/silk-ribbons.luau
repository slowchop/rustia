local kinds = {Sprite.Flower, Sprite.Bud, Sprite.Ring, Sprite.PinkBud, Sprite.BlueFlower}
local knots = {}
function update(time, mouse)
    for i = 0, 95 do
        local ribbon, bead = i // 12, i % 12
        local side = 1 - 2 * (ribbon % 2)
        local home = vector(490 - 195 * side, 85 + (ribbon // 2) * 125)
        local breeze = time * 0.14 - ribbon * 0.38
        local drift = vector(math.noise(breeze, 0.3), math.noise(0.6, breeze + ribbon))
        local tip = home + drift * 175
        local p = knots[i] or tip + vector(side * bead * 15, 0)
        local wave = math.noise(breeze * 1.4 - bead * 0.16, ribbon + 0.5)
        local target = if bead == 0 then tip else knots[i - 1] + vector(side * 15, wave * 13)
        if bead == 0 and mouse then
            local offset = mouse - p
            local distance = vector.magnitude(offset)
            local near = math.max(0, 1 - distance / 300)
            local reach = if ribbon % 3 == 1 then -120 else 120
            local nudge = offset * (reach / math.max(distance, 12))
            target = vector.lerp(tip, p + nudge, near * near)
        end
        knots[i] = p + (target - p) * 0.065
        sprite(knots[i], kinds[ribbon % #kinds + 1])
    end
end
