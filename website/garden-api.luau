setmetatable(Sprite, {__index = function(_, name)
    error("Unknown sprite type: Sprite." .. tostring(name), 2)
end})
table.freeze(Sprite)

local spriteCount = 0
for _ in Sprite do
    spriteCount += 1
end

Flower = table.freeze({
    random = function()
        return math.random(0, spriteCount - 1)
    end,
})
