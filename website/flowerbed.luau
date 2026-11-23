-- Each row carries one flower across a live edit. Restart supplies no rows.
local STRIDE = 7
local flowers = {}
local inUse = false
FlowerBed = {}

function FlowerBed.resize(count)
    assert(count % 1 == 0 and count >= 0 and count <= 512, "Use a whole flower count from 0 to 512.")
    inUse = true
    while #flowers > count do
        table.remove(flowers)
    end
    for i = #flowers + 1, count do
        flowers[i] = {
            position = vector.zero,
            target = vector.zero,
            followsMouse = false,
            speed = math.lerp(0.005, 0.02, i / count),
            kind = Flower.random(),
        }
    end
    return flowers
end

-- The browser passes plain numbers between the old and new Rustia workers.
function FlowerBed.save()
    local rows = {}
    if not inUse then return rows end
    for _, flower in flowers do
        local p, t = flower.position, flower.target
        local row = {p.x, p.y, t.x, t.y, if flower.followsMouse then 1 else 0, flower.speed, flower.kind}
        for _, value in row do
            table.insert(rows, value)
        end
    end
    return rows
end

for i = 1, #previousFlowers, STRIDE do
    flowers[#flowers + 1] = {
        position = vector(previousFlowers[i], previousFlowers[i + 1]),
        target = vector(previousFlowers[i + 2], previousFlowers[i + 3]),
        followsMouse = previousFlowers[i + 4] == 1,
        speed = previousFlowers[i + 5],
        kind = previousFlowers[i + 6],
    }
end
previousFlowers = nil
