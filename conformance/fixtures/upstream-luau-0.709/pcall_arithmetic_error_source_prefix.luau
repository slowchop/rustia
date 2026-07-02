local ok, err = pcall(function()
    local value = nil
    return value + 1
end)

local prefix = "pcall_arithmetic_error_source_prefix.luau:3: "
if not ok and err:sub(1, #prefix) == prefix and err:find("number", 1, true) then
    return "OK"
end

return err
