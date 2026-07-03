local function contains(haystack, needle)
    return string.find(haystack, needle, 1, true) ~= nil
end

local ok, err = pcall(function()
    assert(false, 123)
end)
assert(ok == false and contains(err, "123"))

ok, err = pcall(function()
    assert(false, {})
end)
assert(ok == false and contains(err, "string expected") and contains(err, "table"))

ok, err = pcall(function()
    assert(false, true)
end)
assert(ok == false and contains(err, "string expected") and contains(err, "boolean"))

ok, err = pcall(function()
    assert(false, nil)
end)
assert(ok == false and contains(err, "assertion failed!"))

return "OK"
