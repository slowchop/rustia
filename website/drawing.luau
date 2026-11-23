local draw = draw_sprite
local function finite(value)
    return type(value) == "number" and math.abs(value) < math.huge
end

-- The extra stack level points the error at the user's sprite call.
local function invalid(name, expected, value)
    local actual = if type(value) == "number" then tostring(value) else type(value)
    error("sprite: " .. name .. " must be " .. expected .. ", got " .. actual, 3)
end

-- sprite(position, kind?, rotation?) defaults to a flower with no rotation.
function sprite(position, kind, rotation, legacyAngle)
    local x, y
    if type(position) == "number" then
        -- Saved scripts may pass (kind, position) or (kind, x, y, angle).
        position, kind = kind, position
        if type(position) == "number" then
            x, y, rotation = position, rotation, legacyAngle
        end
    end

    if x == nil then
        local t = type(position)
        if t ~= "vector" and t ~= "table" and t ~= "userdata" then
            invalid("position", "a vector such as vector(100, 50)", position)
        end
        x, y = position.x, position.y
    end

    if kind == nil then kind = Sprite.Flower end
    if not finite(kind) or kind % 1 ~= 0 or kind < -2147483648 or kind > 2147483647 then
        invalid("kind", "a sprite type such as Sprite.Ring (or a 32-bit integer)", kind)
    end
    if not finite(x) then invalid("position.x", "a finite number", x) end
    if not finite(y) then invalid("position.y", "a finite number", y) end
    if rotation == nil then rotation = 0 end
    if not finite(rotation) then invalid("rotation", "a finite number", rotation) end
    draw(kind, x, y, rotation)
end
