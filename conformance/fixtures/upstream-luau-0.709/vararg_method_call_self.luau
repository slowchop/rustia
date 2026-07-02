local t = { 10, 20 }

function t:f(...)
    local index, offset = ...
    return self[index] + offset
end

return t:f(2, 3)
