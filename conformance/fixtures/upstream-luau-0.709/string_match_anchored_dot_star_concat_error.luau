local ok, err = pcall(function()
    return "1" .. nil .. "2"
end)

local message = tostring(ok) .. "," .. err

if message:match("^false,.*attempt to concatenate nil with string") then
    return "OK"
end

return message
