-- Luau 0.709 accepts runtime failures in branches that are never executed.
if false then
    local missing = '' .. tostring()
    local number = '' .. tonumber()
    local kind = '' .. type()
    local raw = '' .. rawget({})
    local insert = '' .. table.insert()
    local math_field = '' .. math.unreal()
    local table_field = '' .. table.unreal()
    local string_field = '' .. string.unreal()
    local extra = '' .. string.len('abc', 99)
end
local function unused()
    return '' .. tostring()
end
return 'ok'
