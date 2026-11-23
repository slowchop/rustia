seed = math.random(1, 999999) -- Set a number here to repeat a race.
local race = require("./demos/web-live/race")
local names = {"Ember", "Mint", "Iris"}

for i = 1, #race, 3 do
    if i > 1 then print("\f") end -- Clear the console for the next frame.
    print("Seed " .. seed)
    local winners = {}
    for lane, name in names do
        local meters = race[i + lane - 1]
        print(string.format("%-5s |%-25s| %3dm", name, string.rep("=", meters // 4), meters))
        if meters == 100 then table.insert(winners, name) end
    end
    if #winners > 0 then
        print(if #winners == 1 then winners[1] .. " wins!" else "Tie: " .. table.concat(winners, ", "))
        break
    end
end
