-- Derived from Luau 0.709 tests/conformance/iter.luau generic for with __call userdata.
local f = newproxy(true)
getmetatable(f).__call = function(_, _, control)
    if control > 0 then
        return control - 1
    end
end

local x = 0
for n in f, nil, 5 do
    x += n
end

return x
