local inext, array, initial = ipairs({5, 6, 7})
local ik, iv = inext(array, 2)

local nextf, state, control = pairs({5, 6, 7})
local pk, pv = nextf(state, control)

if initial == 0 and ik == 3 and iv == 7 and pk == 1 and pv == 5 then
    return "OK"
end

return "bad"
