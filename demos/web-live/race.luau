math.randomseed(seed)

local finish = 100
local distance = {0, 0, 0}
local frames = {0, 0, 0}

while math.min(distance[1], distance[2], distance[3]) < finish do
    for racer = 1, 3 do
        local stride = math.random(1, 6)
        distance[racer] = math.min(finish, distance[racer] + stride)
        table.insert(frames, distance[racer])
    end
end

return frames
