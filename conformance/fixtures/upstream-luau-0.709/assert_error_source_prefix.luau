local ok, err = pcall(function()
    assert(false)
end)

if not ok and err == "assert_error_source_prefix.luau:2: assertion failed!" then
    return "OK"
end

return err
