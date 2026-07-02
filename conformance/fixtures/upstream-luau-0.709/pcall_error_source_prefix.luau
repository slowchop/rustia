local ok, err = pcall(function()
    error("oops")
end)

if not ok and err == "pcall_error_source_prefix.luau:2: oops" then
    return "OK"
end

return err
