local flowerCount = 10
local flowers

-- Keep existing flowers across edits. Only the difference is added or removed.
function setup()
    flowers = FlowerBed.resize(flowerCount)
end

local function randomPointInDisk(radius)
    local angle = math.random() * math.tau
    local distance = math.sqrt(math.random()) * radius
    return vector(math.cos(angle), math.sin(angle)) * distance
end

local function moveFlower(flower, mouse, radius)
    -- Each arrival switches between wandering and following the mouse.
    if vector.magnitude(flower.target - flower.position) < 10 then
        flower.target = randomPointInDisk(radius)
        flower.followsMouse = not flower.followsMouse
    end

    if flower.followsMouse and mouse then
        flower.target = mouse
    end
    flower.position = vector.lerp(flower.position, flower.target, flower.speed)
end

-- Each frame, move the flowers and draw them at their new positions.
function update(time, mouse, radius)
    for _, flower in flowers do
        moveFlower(flower, mouse, radius)
        sprite(flower.position, flower.kind)
    end
end
