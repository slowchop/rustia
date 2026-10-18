local calls = 0
local function extra()
    calls = calls + 1
    return 99
end
local result = 'ipairs='
for key, value in ipairs({4}, extra()) do
    result = result .. key .. ':' .. value
end
result = result .. ';pairs='
for key, value in pairs({4}, extra()) do
    result = result .. key .. ':' .. value
end
return result .. ';calls=' .. calls
