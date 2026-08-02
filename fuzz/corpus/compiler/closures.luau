local function counter(start: number)
    local value = start
    return function(step: number)
        value += step
        return value
    end
end

local next = counter(40)
return next(1) + next(1) - 41

