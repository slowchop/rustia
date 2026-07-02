local ok, err = pcall(function()
    error("hi", 0)
end)

if not ok then
    return err
end

return "bad"
