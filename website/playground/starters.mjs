export const interactiveStarters = {
  guess: {'main.luau':`local answer = math.random(1, 10)
print("Guess a number from 1 to 10.")

while true do
    local guess = tonumber(readLine())
    if guess == answer then
        print("You got it!")
        break
    elseif not guess then
        print("Type a number.")
    elseif guess < answer then
        print("Higher.")
    else
        print("Lower.")
    end
end
`},
  adventure: {
    'main.luau':`local rooms = require("./rooms")
local location = "trail"
local hasKey = false

while location do
    local room = rooms[location]
    print(room.description)
    local command = string.lower(readLine())

    if command == "take key" and location == "hut" then
        hasKey = true
        print("You pocket the key.")
    elseif command == "open gate" and location == "gate" then
        if hasKey then
            print("The key turns. You follow the path home.")
            break
        end
        print("The gate is locked.")
    elseif room.exits[command] then
        location = room.exits[command]
    elseif command == "quit" then
        break
    else
        print("Try north, south, east, west, take key, open gate, or quit.")
    end
end
`,
    'rooms.luau':`return {
    trail = {
        description = "The track forks. A hut is west. A gate is north.",
        exits = { west = "hut", north = "gate" },
    },
    hut = {
        description = "A key lies on a bench. The track is east.",
        exits = { east = "trail" },
    },
    gate = {
        description = "A locked gate blocks the path home. The track is south.",
        exits = { south = "trail" },
    },
}
`,
  },
};
